use std::{
    collections::HashMap,
    io::{Read, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex, OnceLock},
};

use serde_json::{json, Value};

use crate::os::{events, security};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "exec" => exec(args, false),
        "execShell" => exec(args, true),
        "spawn" => spawn(args),
        "writeStdin" => write_stdin(args),
        "closeStdin" => close_stdin(args),
        "kill" => kill(args),
        _ => Err(format!("process.{method}: unknown method")),
    }
}

fn exec(args: &Value, shell: bool) -> Result<Value, String> {
    let cmd_str = args
        .get("cmd")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "process.exec: missing 'cmd'".to_string())?;

    let argv: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // `execShell` mode runs `cmd` through the system shell, which interprets
    // metacharacters. Naively appending caller-controlled `args` would let any
    // value containing spaces, `&&`, `;`, `|`, or backticks inject extra
    // commands. Refuse `args` in shell mode so callers must embed everything
    // they need into the `cmd` string explicitly.
    if shell && !argv.is_empty() {
        return Err(
            "process.execShell: extra 'args' are not allowed in shell mode — embed them in the \
             'cmd' string yourself (and quote them)"
                .into(),
        );
    }

    // SECURITY: shell execution requires explicit
    // `security.configure({ shellExecAllowedCommands: [...] })`. Without
    // it, all calls reject with `process.shellExec: not permitted`. The
    // allowlist gates the program name and the call rejects any shell
    // metacharacter so a whitelisted `ls` can't smuggle `; rm -rf /`.
    // The plain `exec` path (separate `args`, no shell) is always
    // available and is the safe way to run subprocesses.
    if shell {
        security::check_shell_exec(cmd_str)?;
    }

    let mut command = if shell {
        let joined = cmd_str.to_string();
        #[cfg(target_os = "windows")]
        {
            let mut c = Command::new("cmd");
            c.args(["/C", &joined]);
            c
        }
        #[cfg(not(target_os = "windows"))]
        {
            let mut c = Command::new("sh");
            c.args(["-c", &joined]);
            c
        }
    } else {
        let mut c = Command::new(cmd_str);
        c.args(&argv);
        c
    };

    if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
        // SECURITY: route `cwd` through the same fs allow-list as every
        // other host file access. Without this, a caller can pin a child
        // process to `/etc` (Unix) or `C:\Windows\System32` (Windows) and
        // launch a binary that reads from there using relative paths,
        // sidestepping the security policy.
        security::check_fs(cwd)
            .map_err(|e| format!("process.exec: cwd not permitted: {e}"))?;
        command.current_dir(cwd);
    }

    if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
        let merged: HashMap<String, String> = env_obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        command.envs(merged);
    }

    let input = args.get("input").and_then(|v| v.as_str()).map(String::from);

    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| format!("spawn failed: {e}"))?;

    if let Some(data) = input {
        if let Some(mut stdin) = child.stdin.take() {
            // Kill the child if stdin write fails — otherwise it hangs on
            // `wait_with_output` with a half-filled stdin pipe and the
            // process leaks until the OS cleans it up.
            if let Err(e) = stdin.write_all(data.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("stdin write failed: {e}"));
            }
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait failed: {e}"))?;

    Ok(json!({
        "code": output.status.code(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

// ─── Streaming subprocess (`process.spawn`) ────────────────────────────
//
// `spawn` returns immediately with a PID. Three background threads then
// pump stdout / stderr / exit into events the frontend `SpawnedProcess`
// proxy subscribes to:
//
//   process:stdout:<pid>  { data: number[] }   // chunk of bytes
//   process:stderr:<pid>  { data: number[] }
//   process:exit:<pid>    { code, signal }     // terminal event
//
// `data` is encoded as a JSON array of byte values rather than base64 so
// the frontend can reconstruct a `Uint8Array` without a base64 round-trip
// on every chunk. The `process` registry holds the `Child` until exit so
// `kill` and `writeStdin` can reach it from the call-pool thread.

/// Per-pid handles kept around for runtime mutations. `Child` lives until
/// the waiter thread observes exit, then drops it from the registry.
struct SpawnEntry {
    child: Mutex<Option<Child>>,
    /// Owned `ChildStdin` so `writeStdin` calls can reuse the pipe across
    /// multiple frames. `None` once the user calls `closeStdin` (or stdin
    /// was never piped).
    stdin: Mutex<Option<ChildStdin>>,
}

static REGISTRY: OnceLock<Mutex<HashMap<u32, Arc<SpawnEntry>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<u32, Arc<SpawnEntry>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn registry_lock() -> std::sync::MutexGuard<'static, HashMap<u32, Arc<SpawnEntry>>> {
    registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn registry_remove(pid: u32) -> Option<Arc<SpawnEntry>> {
    registry_lock().remove(&pid)
}

fn registry_get(pid: u32) -> Option<Arc<SpawnEntry>> {
    registry_lock().get(&pid).cloned()
}

fn require_pid(args: &Value, op: &str) -> Result<u32, String> {
    let v = args
        .get("pid")
        .ok_or_else(|| format!("process.{op}: missing 'pid'"))?;
    v.as_u64()
        .map(|n| n as u32)
        .ok_or_else(|| format!("process.{op}: 'pid' must be a number"))
}

/// Read one bytes chunk and emit it as `process:<stream>:<pid>`. Stops on
/// EOF or read error. We avoid the per-byte JSON cost by emitting the
/// whole chunk once per read — typical pipe reads return up to 64 KiB at
/// once on every platform, so this is a small constant number of events
/// per second on a busy stream rather than one per byte.
fn pump_stream<R: Read>(mut reader: R, event_name: String) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF — child closed the stream.
            Ok(n) => {
                // serde_json encodes `&[u8]` as a `Number[]` array, which is
                // what the frontend `process.spawn` proxy expects (it
                // rebuilds a `Uint8Array` from the array). A typed array
                // wire format would shave allocations but JSON keeps the
                // path identical to every other OS event.
                let chunk: Vec<Value> = buf[..n].iter().map(|b| Value::from(u32::from(*b))).collect();
                events::emit(&event_name, &json!({ "data": Value::Array(chunk) }));
            },
            Err(e) => {
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                // Non-fatal — emit a final empty chunk and bail. The waiter
                // thread will still publish the exit event so the frontend
                // iterator terminates cleanly.
                crate::tynd_log!("process: {event_name} read error: {e}");
                break;
            },
        }
    }
}

fn spawn(args: &Value) -> Result<Value, String> {
    let cmd_str = args
        .get("cmd")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "process.spawn: missing 'cmd'".to_string())?;

    let argv: Vec<String> = args
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| a.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut command = Command::new(cmd_str);
    command.args(&argv);

    if let Some(cwd) = args.get("cwd").and_then(|v| v.as_str()) {
        // SECURITY: same gate as `exec` — `cwd` becomes the child's working
        // directory and any relative path it dereferences is resolved
        // against this. Run it through the fs policy before honoring it.
        security::check_fs(cwd)
            .map_err(|e| format!("process.spawn: cwd not permitted: {e}"))?;
        command.current_dir(cwd);
    }
    if let Some(env_obj) = args.get("env").and_then(|v| v.as_object()) {
        let merged: HashMap<String, String> = env_obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        command.envs(merged);
    }

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|e| format!("process.spawn: {e}"))?;

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdin = child.stdin.take();

    // Refcounted entry: the registry, the waiter thread, and any
    // concurrent `kill`/`writeStdin` callers each hold their own `Arc`.
    // The last drop frees the entry — no race between `kill()` and the
    // waiter's `Box::from_raw` (the original use-after-free).
    let entry = Arc::new(SpawnEntry {
        child: Mutex::new(Some(child)),
        stdin: Mutex::new(stdin),
    });

    // stdout reader
    if let Some(out) = stdout {
        let event_name = format!("process:stdout:{pid}");
        std::thread::spawn(move || pump_stream(out, event_name));
    }
    // stderr reader
    if let Some(err) = stderr {
        let event_name = format!("process:stderr:{pid}");
        std::thread::spawn(move || pump_stream(err, event_name));
    }

    registry_lock().insert(pid, Arc::clone(&entry));

    // Waiter thread — `wait()` blocks until the child exits, which is
    // exactly what we want here. The OS already gave us a separate handle
    // per stdio pipe, so the read threads above are unaffected.
    let waiter_entry = Arc::clone(&entry);
    std::thread::spawn(move || {
        // Take the Child out of the entry so we can call `wait()`
        // without holding the mutex (long blocking call). A racing
        // `kill()` that grabs the lock before we do still works — it
        // just sends the signal first; our `wait()` then reaps it.
        let child_opt = {
            let mut guard = waiter_entry
                .child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.take()
        };
        if let Some(mut c) = child_opt {
            let status = c.wait();
            let (code, signal) = match status {
                Ok(s) => {
                    let code = s.code();
                    #[cfg(unix)]
                    let signal = {
                        use std::os::unix::process::ExitStatusExt as _;
                        s.signal().map(|n| format!("SIG{n}"))
                    };
                    #[cfg(not(unix))]
                    let signal: Option<String> = None;
                    (code, signal)
                },
                Err(_) => (None, None),
            };
            events::emit(
                &format!("process:exit:{pid}"),
                &json!({
                    "code": code,
                    "signal": signal,
                }),
            );
        }
        // Drop the registry's Arc. Any caller that grabbed an Arc in
        // between (a racing `writeStdin` / `kill`) keeps the entry
        // alive until *they* drop their clone — the entry is freed on
        // the last decrement, which is the safe inverse of the old
        // `Box::leak` / `Box::from_raw` dance.
        let _ = registry_remove(pid);
        // `waiter_entry` itself drops here.
        drop(waiter_entry);
    });

    Ok(json!({ "pid": pid }))
}

fn write_stdin(args: &Value) -> Result<Value, String> {
    let pid = require_pid(args, "writeStdin")?;
    // Accept either a string ("data" as utf-8) or an array of bytes.
    let bytes: Vec<u8> = if let Some(s) = args.get("data").and_then(Value::as_str) {
        s.as_bytes().to_vec()
    } else if let Some(arr) = args.get("data").and_then(Value::as_array) {
        arr.iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| "process.writeStdin: 'data' bytes out of range".to_string())
            })
            .collect::<Result<Vec<u8>, String>>()?
    } else {
        return Err("process.writeStdin: 'data' must be string or byte array".into());
    };

    let entry = registry_get(pid)
        .ok_or_else(|| format!("process.writeStdin: pid {pid} not found (already exited?)"))?;
    let mut stdin_opt = entry
        .stdin
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stdin = stdin_opt
        .as_mut()
        .ok_or_else(|| format!("process.writeStdin: stdin closed for pid {pid}"))?;
    stdin
        .write_all(&bytes)
        .map_err(|e| format!("process.writeStdin: {e}"))?;
    Ok(Value::Null)
}

fn close_stdin(args: &Value) -> Result<Value, String> {
    let pid = require_pid(args, "closeStdin")?;
    let entry = registry_get(pid)
        .ok_or_else(|| format!("process.closeStdin: pid {pid} not found"))?;
    // Dropping the ChildStdin closes the write end of the pipe — the child
    // sees EOF on stdin which is exactly what users mean by "close stdin".
    *entry
        .stdin
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    Ok(Value::Null)
}

fn kill(args: &Value) -> Result<Value, String> {
    let pid = require_pid(args, "kill")?;
    let signal = args
        .get("signal")
        .and_then(Value::as_str)
        .unwrap_or("SIGTERM");

    let entry = registry_get(pid)
        .ok_or_else(|| format!("process.kill: pid {pid} not found"))?;

    // On Unix we honour the actual signal name — `SIGTERM` and `SIGINT`
    // give the child a chance to shut down gracefully (close sockets,
    // flush buffers); only `SIGKILL` (or the `Child::kill` fallback)
    // takes the kernel-level prejudice path. On Windows there's no
    // signal abstraction — `TerminateProcess` is the only knob, so the
    // signal name is recorded for forward-compat but otherwise ignored.
    #[cfg(unix)]
    {
        use nix::{sys::signal::Signal, unistd::Pid};
        // `Child::kill()` always sends SIGKILL — for SIGTERM/SIGINT we
        // go through `nix::sys::signal::kill`. We hold the child mutex
        // only long enough to learn the pid; the actual signal goes
        // through the OS by pid number so a racing `wait()` doesn't
        // deadlock on the lock.
        let target_pid: Option<i32> = {
            let guard = entry
                .child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.as_ref().map(|c| c.id() as i32)
        };
        let Some(target_pid) = target_pid else {
            // Already reaped by the waiter thread.
            return Ok(Value::Null);
        };
        let sig = match signal {
            "SIGINT" => Signal::SIGINT,
            "SIGHUP" => Signal::SIGHUP,
            "SIGQUIT" => Signal::SIGQUIT,
            "SIGUSR1" => Signal::SIGUSR1,
            "SIGUSR2" => Signal::SIGUSR2,
            "SIGKILL" => Signal::SIGKILL,
            // SIGTERM is the documented default; an unknown name falls
            // through to it rather than escalating straight to SIGKILL
            // so callers get the polite shutdown they asked for.
            _ => Signal::SIGTERM,
        };
        match nix::sys::signal::kill(Pid::from_raw(target_pid), sig) {
            Ok(()) => {},
            // ESRCH (pid already exited) is benign — the waiter will
            // emit the exit event when it observes the reap.
            Err(nix::errno::Errno::ESRCH) => {},
            Err(e) => return Err(format!("process.kill: {e}")),
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: TerminateProcess is the only termination primitive;
        // record the requested signal in the log so callers can see
        // their preference was acknowledged but treat all values as a
        // hard kill.
        let _ = signal;
        let mut child_opt = entry
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(child) = child_opt.as_mut() {
            child
                .kill()
                .map_err(|e| format!("process.kill: {e}"))?;
        }
    }
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os::security;

    /// Reset the global shell-exec whitelist after each test so nothing
    /// leaks into the `default-deny` assertions below.
    struct ShellExecGuard;
    impl Drop for ShellExecGuard {
        fn drop(&mut self) {
            security::set_shell_exec_allowed_for_tests(Vec::new());
        }
    }

    fn allow_shell(commands: &[&str]) -> ShellExecGuard {
        security::set_shell_exec_allowed_for_tests(
            commands.iter().map(|s| (*s).to_string()).collect(),
        );
        ShellExecGuard
    }

    #[test]
    fn echo_captures_stdout() {
        #[cfg(target_os = "windows")]
        let v = exec(
            &json!({ "cmd": "cmd", "args": ["/C", "echo hello"] }),
            false,
        )
        .unwrap();
        #[cfg(not(target_os = "windows"))]
        let v = exec(&json!({ "cmd": "echo", "args": ["hello"] }), false).unwrap();
        assert!(v["stdout"].as_str().unwrap().contains("hello"));
        assert_eq!(v["code"].as_i64().unwrap(), 0);
    }

    #[test]
    fn exec_shell_runs_simple_command_when_whitelisted() {
        let _g = security::test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = allow_shell(&["echo"]);
        let v = exec(&json!({ "cmd": "echo piped" }), true).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(stdout.contains("piped"), "expected stdout to contain 'piped', got: {stdout}");
    }

    #[test]
    fn exec_shell_rejects_when_not_configured() {
        let _g = security::test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // No whitelist set — every call must reject.
        security::set_shell_exec_allowed_for_tests(Vec::new());
        let err = exec(&json!({ "cmd": "ls" }), true).unwrap_err();
        assert!(
            err.contains("not permitted"),
            "expected 'not permitted' error, got: {err}"
        );
    }

    #[test]
    fn exec_shell_rejects_unlisted_command() {
        let _g = security::test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = allow_shell(&["echo"]);
        let err = exec(&json!({ "cmd": "rm -rf /tmp/foo" }), true).unwrap_err();
        assert!(
            err.contains("not in `shellExecAllowedCommands`"),
            "expected whitelist-miss error, got: {err}"
        );
    }

    #[test]
    fn exec_shell_rejects_metacharacter_even_if_whitelisted() {
        let _g = security::test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _reset = allow_shell(&["ls"]);
        // `ls` is whitelisted but `;` is a shell metacharacter — refuse.
        let err = exec(&json!({ "cmd": "ls; rm -rf /" }), true).unwrap_err();
        assert!(
            err.contains("metacharacters are not allowed"),
            "expected metachar-reject error, got: {err}"
        );
        // Pipe is also a metacharacter.
        let err = exec(&json!({ "cmd": "ls | cat" }), true).unwrap_err();
        assert!(err.contains("metacharacters"));
    }

    #[test]
    fn missing_cmd_errors() {
        assert!(exec(&json!({}), false).is_err());
    }

    #[test]
    fn nonzero_exit_preserved() {
        #[cfg(target_os = "windows")]
        let v = exec(&json!({ "cmd": "cmd", "args": ["/C", "exit 7"] }), false).unwrap();
        #[cfg(not(target_os = "windows"))]
        let v = exec(&json!({ "cmd": "sh", "args": ["-c", "exit 7"] }), false).unwrap();
        assert_eq!(v["code"].as_i64().unwrap(), 7);
    }

    /// `spawn` returns immediately with a numeric pid that lands in the
    /// global registry. We don't have a live event-loop in tests so we
    /// can't assert on the emitted `process:stdout:<pid>` events directly,
    /// but the registry insertion + a successful `kill` round trip is
    /// enough to exercise the bookkeeping path.
    #[test]
    fn spawn_returns_pid_and_registers_entry() {
        #[cfg(target_os = "windows")]
        let v = spawn(&json!({
            "cmd": "cmd",
            "args": ["/C", "ping", "-n", "5", "127.0.0.1"]
        }))
        .expect("spawn");
        #[cfg(not(target_os = "windows"))]
        let v = spawn(&json!({
            "cmd": "sleep",
            "args": ["2"]
        }))
        .expect("spawn");

        let pid = v["pid"].as_u64().expect("pid is u64") as u32;
        assert!(pid > 0);
        assert!(registry_lock().contains_key(&pid));

        // Kill it so the test doesn't leak processes.
        kill(&json!({ "pid": pid })).expect("kill");

        // Give the waiter thread a moment to drain — exit emit + registry
        // remove happens off the dispatch thread.
        for _ in 0..50 {
            if !registry_lock().contains_key(&pid) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // If we get here the waiter never reaped the entry. Not a hard
        // failure (the pid may still be alive if the OS is slow), but
        // worth flagging.
        assert!(
            !registry_lock().contains_key(&pid),
            "spawn waiter never removed pid {pid} from registry"
        );
    }

    #[test]
    fn spawn_invalid_binary_rejects() {
        let err = spawn(&json!({
            "cmd": "this-binary-does-not-exist-tynd-test-xyz",
            "args": []
        }))
        .unwrap_err();
        assert!(err.contains("process.spawn"), "got: {err}");
    }

    #[test]
    fn write_stdin_unknown_pid_errors() {
        let err = write_stdin(&json!({ "pid": 99999999u64, "data": "hi" })).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn kill_unknown_pid_errors() {
        let err = kill(&json!({ "pid": 99999999u64 })).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn require_pid_validates() {
        assert!(require_pid(&json!({}), "writeStdin").is_err());
        assert!(require_pid(&json!({ "pid": "abc" }), "writeStdin").is_err());
        assert_eq!(require_pid(&json!({ "pid": 42 }), "writeStdin").unwrap(), 42);
    }
}
