use std::{
    io::{Read, Write},
    process::{ChildStderr, ChildStdout, Command, ExitStatus, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use base64::Engine as _;
use serde_json::{json, Value};
use shared_child::SharedChild;

use super::{prepare_command, proc_err, registry};
use crate::ipc::owner;

/// How long a spawned child may wait for the renderer's `activate` call before
/// it is killed and reaped. Covers a webview that reloads or closes between
/// the spawn reply and `activate`: without a bound the child, its pipes and
/// its registry slot would be held until process exit.
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Read a chunk and emit `process:<stream>:<pid>`. One event per read (up to
/// 64 KiB) instead of one per byte.
///
/// Wire: `{ data: "<base64>" }` (decode via `atob` -> `Uint8Array.from`).
fn pump_stream<R: Read>(
    mut reader: R,
    entry: &registry::SpawnEntry,
    event_name: String,
    end_event_name: String,
) {
    let mut buf = vec![0u8; 65_536];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                entry.emit(&event_name, &json!({ "data": chunk }));
            },
            Err(e) => {
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                crate::tynd_warn!("process: {event_name} read error: {e}");
                break;
            },
        }
    }
    entry.emit(&end_event_name, &json!({}));
}

pub(super) fn spawn_impl(
    cmd_str: &str,
    argv: Vec<String>,
    cwd: Option<String>,
    env: Option<serde_json::Map<String, Value>>,
) -> Result<Value, String> {
    let _admission = registry::admit_spawn()?;
    let mut command = Command::new(cmd_str);
    command.args(&argv);
    prepare_command(&mut command, "spawn", cwd.as_deref(), env.as_ref())?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let child = spawn_managed_child(&mut command)?;
    start_stream_pump(child.stdout, child.pid, "stdout", &child.entry)?;
    start_stream_pump(child.stderr, child.pid, "stderr", &child.entry)?;
    registry::insert(child.pid, Arc::clone(&child.entry));
    // The spawning webview may have committed a new document while the child
    // was starting; the process is then backend-owned, not the next document's.
    child.entry.reown_if_stale();
    start_waiter(child.pid, &child.entry)?;
    Ok(json!({ "pid": child.pid }))
}

struct SpawnedChild {
    pid: u32,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    entry: Arc<registry::SpawnEntry>,
}

fn spawn_managed_child(command: &mut Command) -> Result<SpawnedChild, String> {
    registry::reserve_slot()?;
    let mut raw = match command.spawn() {
        Ok(raw) => raw,
        Err(error) => {
            registry::release_slot();
            return Err(proc_err("spawn", error));
        },
    };
    let pid = raw.id();
    let stdout = raw.stdout.take();
    let stderr = raw.stderr.take();
    let stdin = raw.stdin.take();
    let child = match SharedChild::new(raw) {
        Ok(child) => Arc::new(child),
        Err(error) => {
            registry::release_slot();
            return Err(proc_err("spawn", format!("SharedChild::new: {error}")));
        },
    };
    Ok(SpawnedChild {
        pid,
        stdout,
        stderr,
        entry: registry::new_entry(child, stdin, owner::current()),
    })
}

fn start_stream_pump<R: Read + Send + 'static>(
    reader: Option<R>,
    pid: u32,
    stream: &'static str,
    entry: &Arc<registry::SpawnEntry>,
) -> Result<(), String> {
    let Some(reader) = reader else {
        return Ok(());
    };
    let event_name = format!("process:{stream}:{pid}");
    let end_event_name = format!("process:{stream}-end:{pid}");
    let worker_entry = Arc::clone(entry);
    if let Err(error) = thread::Builder::new()
        .name(format!("tynd-process-{stream}-{pid}"))
        .spawn(move || {
            worker_entry.wait_until_active();
            pump_stream(reader, &worker_entry, event_name, end_event_name);
        })
    {
        terminate_unregistered(entry);
        return Err(proc_err(
            "spawn",
            format!("failed to spawn {stream} pump: {error}"),
        ));
    }
    Ok(())
}

fn start_waiter(pid: u32, entry: &Arc<registry::SpawnEntry>) -> Result<(), String> {
    let waiter_entry = Arc::clone(entry);
    if let Err(error) = thread::Builder::new()
        .name(format!("tynd-process-wait-{pid}"))
        .spawn(move || {
            if !waiter_entry.wait_until_active_for(ACTIVATION_TIMEOUT) {
                crate::tynd_warn!(
                    "process: pid {pid} was never activated within {}s; killing it",
                    ACTIVATION_TIMEOUT.as_secs()
                );
                let _ = waiter_entry.child.kill();
                // Release the pump threads so they drain the closed pipes.
                waiter_entry.activate();
            }
            emit_exit(&waiter_entry, pid, waiter_entry.child.wait());
            let _ = registry::remove(pid);
            registry::release_slot();
            drop(waiter_entry);
        })
    {
        let _ = registry::remove(pid);
        terminate_unregistered(entry);
        return Err(proc_err(
            "spawn",
            format!("failed to spawn child waiter: {error}"),
        ));
    }
    Ok(())
}

fn emit_exit(entry: &registry::SpawnEntry, pid: u32, status: std::io::Result<ExitStatus>) {
    let (code, signal, error) = match status {
        Ok(status) => {
            let code = status.code();
            #[cfg(unix)]
            let signal = {
                use std::os::unix::process::ExitStatusExt as _;
                status.signal().map(|number| format!("SIG{number}"))
            };
            #[cfg(not(unix))]
            let signal: Option<String> = None;
            (code, signal, None)
        },
        Err(error) => (None, None, Some(error.to_string())),
    };
    entry.emit(
        &format!("process:exit:{pid}"),
        &json!({
            "code": code,
            "signal": signal,
            "error": error,
        }),
    );
}

fn terminate_unregistered(entry: &registry::SpawnEntry) {
    entry.activate();
    let _ = entry.child.kill();
    let _ = entry.child.wait();
    registry::release_slot();
}

pub(super) fn activate_impl(pid: u32) -> Result<(), String> {
    let entry = registry::get_for_caller(pid)
        .ok_or_else(|| proc_err("activate", format!("pid {pid} not found (already exited?)")))?;
    entry.activate();
    Ok(())
}

pub(super) fn write_stdin_impl(pid: u32, bytes: &[u8]) -> Result<(), String> {
    let entry = registry::get_for_caller(pid).ok_or_else(|| {
        proc_err(
            "writeStdin",
            format!("pid {pid} not found (already exited?)"),
        )
    })?;
    let mut stdin_opt = entry.stdin.lock();
    let stdin = stdin_opt
        .as_mut()
        .ok_or_else(|| proc_err("writeStdin", format!("stdin closed for pid {pid}")))?;
    stdin
        .write_all(bytes)
        .map_err(|e| proc_err("writeStdin", e))?;
    Ok(())
}

pub(super) fn close_stdin_impl(pid: u32) -> Result<(), String> {
    let entry = registry::get_for_caller(pid)
        .ok_or_else(|| proc_err("closeStdin", format!("pid {pid} not found")))?;
    *entry.stdin.lock() = None;
    Ok(())
}

pub(super) fn kill_impl(pid: u32, signal: &str) -> Result<(), String> {
    let entry = registry::get_for_caller(pid)
        .ok_or_else(|| proc_err("kill", format!("pid {pid} not found")))?;
    // Cancellation must also release a not-yet-activated reservation so its
    // waiter can reap the child and return the bounded registry slot.
    entry.activate();

    #[cfg(unix)]
    {
        use shared_child::unix::SharedChildExt as _;
        let sig = match signal {
            "SIGINT" => libc::SIGINT,
            "SIGHUP" => libc::SIGHUP,
            "SIGQUIT" => libc::SIGQUIT,
            "SIGUSR1" => libc::SIGUSR1,
            "SIGUSR2" => libc::SIGUSR2,
            "SIGKILL" => libc::SIGKILL,
            _ => libc::SIGTERM,
        };
        // `send_signal` holds the child's state lock and is a no-op once the
        // child has been reaped, so a recycled pid can never be signalled.
        match entry.child.send_signal(sig) {
            Ok(()) => {},
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => {},
            Err(e) => return Err(proc_err("kill", e)),
        }
    }
    #[cfg(not(unix))]
    {
        // Windows has no per-process SIGINT: `CTRL_C_EVENT` cannot target a
        // process group, `CTRL_BREAK_EVENT` needs a shared console (the host
        // is a GUI-subsystem app and children are created without one), so
        // every signal name maps to `TerminateProcess`.
        let _ = signal;
        entry.child.kill().map_err(|e| proc_err("kill", e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn sleeper() -> (&'static str, Vec<String>) {
        if cfg!(windows) {
            ("ping", vec!["-n".into(), "30".into(), "127.0.0.1".into()])
        } else {
            ("sleep", vec!["30".into()])
        }
    }

    fn wait_until_gone(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while registry::get(pid).is_some() {
            assert!(Instant::now() < deadline, "pid {pid} was never reaped");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn spawn_records_the_calling_webview_as_owner() {
        let (command, argv) = sleeper();
        let owned = owner::with_caller("panel", || spawn_impl(command, argv.clone(), None, None))
            .expect("spawn");
        let owned_pid = u32::try_from(owned["pid"].as_u64().unwrap()).unwrap();
        let backend = spawn_impl(command, argv, None, None).expect("spawn");
        let backend_pid = u32::try_from(backend["pid"].as_u64().unwrap()).unwrap();

        assert_eq!(
            registry::get(owned_pid).unwrap().owner().as_deref(),
            Some("panel")
        );
        assert_eq!(registry::get(backend_pid).unwrap().owner(), None);

        for pid in [owned_pid, backend_pid] {
            kill_impl(pid, "SIGKILL").expect("kill");
            wait_until_gone(pid);
        }
    }

    #[test]
    fn spawn_after_the_document_was_replaced_is_backend_owned() {
        let label = "proc-late";
        let (command, argv) = sleeper();
        let spawned = owner::with_caller(label, || {
            // The commit lands while the spawn call is still running.
            owner::bump_generation(label);
            spawn_impl(command, argv, None, None)
        })
        .expect("spawn");
        let pid = u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap();

        assert_eq!(registry::get(pid).unwrap().owner(), None);
        // The next document of the label is remote: it cannot reach it.
        owner::with_caller_trust(label, false, || {
            assert!(registry::get_for_caller(pid).is_none());
        });
        kill_impl(pid, "SIGKILL").expect("kill");
        wait_until_gone(pid);
    }

    #[test]
    fn remote_iframe_cannot_control_the_local_pages_process() {
        let (command, argv) = sleeper();
        let spawned = owner::with_caller("proc-frame", || spawn_impl(command, argv, None, None))
            .expect("spawn");
        let pid = u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap();
        owner::with_caller_trust("proc-frame", false, || {
            assert!(registry::get_for_caller(pid).is_none());
            assert!(kill_impl(pid, "SIGKILL").is_err());
        });
        assert!(registry::get(pid).is_some());
        owner::with_caller("proc-frame", || kill_impl(pid, "SIGKILL")).expect("owner kill");
        wait_until_gone(pid);
    }

    /// Error text with the pid masked, to compare refused vs unknown ids.
    fn masked(error: String, pid: u32) -> String {
        error.replace(&pid.to_string(), "<pid>")
    }

    #[test]
    fn id_based_ops_by_another_webview_answer_like_an_unknown_pid() {
        let (command, argv) = sleeper();
        let spawned =
            owner::with_caller("proc-a", || spawn_impl(command, argv, None, None)).expect("spawn");
        let pid = u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap();
        let unknown = u32::MAX - 1;

        owner::with_caller("proc-b", || {
            assert_eq!(
                masked(activate_impl(pid).unwrap_err(), pid),
                masked(activate_impl(unknown).unwrap_err(), unknown)
            );
            assert_eq!(
                masked(write_stdin_impl(pid, b"x").unwrap_err(), pid),
                masked(write_stdin_impl(unknown, b"x").unwrap_err(), unknown)
            );
            assert_eq!(
                masked(close_stdin_impl(pid).unwrap_err(), pid),
                masked(close_stdin_impl(unknown).unwrap_err(), unknown)
            );
            assert_eq!(
                masked(kill_impl(pid, "SIGKILL").unwrap_err(), pid),
                masked(kill_impl(unknown, "SIGKILL").unwrap_err(), unknown)
            );
        });
        // Nothing above touched the process: still registered, stdin open.
        let entry = registry::get(pid).expect("still registered");
        assert!(entry.stdin.lock().is_some());

        // The owner and the backend may operate on it.
        owner::with_caller("proc-a", || activate_impl(pid)).expect("owner activate");
        write_stdin_impl(pid, b"").expect("backend write");
        owner::with_caller("proc-a", || close_stdin_impl(pid)).expect("owner close");
        kill_impl(pid, "SIGKILL").expect("backend kill");
        wait_until_gone(pid);
    }

    #[test]
    fn owner_may_kill_its_own_process() {
        let (command, argv) = sleeper();
        let spawned =
            owner::with_caller("proc-c", || spawn_impl(command, argv, None, None)).expect("spawn");
        let pid = u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap();
        owner::with_caller("proc-c", || kill_impl(pid, "SIGKILL")).expect("owner kill");
        wait_until_gone(pid);
    }

    #[test]
    fn release_reowns_processes_to_the_backend_and_keeps_them_running() {
        let (command, argv) = sleeper();
        let spawn = |label: &str| {
            let spawned =
                owner::with_caller(label, || spawn_impl(command, argv.clone(), None, None))
                    .expect("spawn");
            u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap()
        };
        let released = spawn("proc-rel-a");
        let kept = spawn("proc-rel-b");

        registry::release_owner("proc-rel-a");

        let entry = registry::get(released).expect("still running");
        assert_eq!(entry.owner(), None);
        assert_eq!(
            registry::get(kept).unwrap().owner().as_deref(),
            Some("proc-rel-b")
        );
        // A later document of the same label is no longer the owner, but a
        // local-trust one reaches it as a backend resource; a remote one does not.
        owner::with_caller_trust("proc-rel-a", true, || {
            assert!(registry::get_for_caller(released).is_some());
        });
        owner::with_caller_trust("proc-rel-a", false, || {
            assert!(registry::get_for_caller(released).is_none());
            assert_eq!(
                masked(kill_impl(released, "SIGKILL").unwrap_err(), released),
                masked(
                    kill_impl(u32::MAX - 1, "SIGKILL").unwrap_err(),
                    u32::MAX - 1
                )
            );
        });
        assert!(registry::get(released).is_some());

        for pid in [released, kept] {
            kill_impl(pid, "SIGKILL").expect("kill");
            wait_until_gone(pid);
        }
    }

    #[test]
    fn kill_of_unknown_pid_is_an_error() {
        let error = kill_impl(u32::MAX - 1, "SIGTERM").expect_err("unknown pid");
        assert!(error.contains("not found"), "{error}");
    }

    #[test]
    fn kill_reaps_a_never_activated_child_and_frees_its_registry_slot() {
        let (command, argv) = sleeper();
        let spawned = spawn_impl(command, argv, None, None).expect("spawn");
        let pid = u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap();
        assert!(registry::get(pid).is_some());

        // No `activate`: kill must release the reservation itself so the
        // waiter reaps the child instead of idling out the 30 s deadline.
        kill_impl(pid, "SIGKILL").expect("kill");
        wait_until_gone(pid);

        let error = kill_impl(pid, "SIGKILL").expect_err("already reaped");
        assert!(error.contains("not found"), "{error}");
    }

    #[test]
    fn unknown_signal_name_still_terminates_the_child() {
        let (command, argv) = sleeper();
        let spawned = spawn_impl(command, argv, None, None).expect("spawn");
        let pid = u32::try_from(spawned["pid"].as_u64().unwrap()).unwrap();
        activate_impl(pid).expect("activate");
        kill_impl(pid, "SIGBOGUS").expect("kill");
        wait_until_gone(pid);
    }
}
