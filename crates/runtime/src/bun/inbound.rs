//! Bun output transport: read logs and frames, decode events, and dispatch trusted host calls.

use std::{
    io::{BufRead, BufReader},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
};

use crossbeam_channel as mpsc;
use serde_json::Value;
use tynd_host::{tynd_info, tynd_warn};
use tynd_protocol::runtime::{BackendCall, BackendConfig, BackendEvent, BackendReturnPayload};

use super::wire::{
    parse_bun_msg, read_bounded_json_line, read_bounded_line, read_config, strip_trailing_eol,
    BunMsg,
};
use super::ChildSlot;

const MAX_LOG_LINE: usize = 64 * 1024;
pub(super) struct StdoutReaderContext {
    pub(super) event_tx: mpsc::Sender<BackendEvent>,
    pub(super) call_tx: mpsc::Sender<BackendCall>,
    pub(super) config_tx: mpsc::Sender<Result<BackendConfig, String>>,
    pub(super) shutdown_expected: Arc<AtomicBool>,
    pub(super) packaged_smoke: bool,
    /// Weak on purpose: a strong reference held by a reader blocked on a live
    /// child would stop `ChildSlot::drop` from ever killing it.
    pub(super) child_slot: Weak<ChildSlot>,
    pub(super) stdin_slot: Arc<Mutex<Option<ChildStdin>>>,
}

/// One backend-to-host call, answered off the stdout reader thread.
struct HostCallJob {
    id: String,
    api: String,
    method: String,
    args: Value,
}

/// Sends `HostCallJob`s to a worker so a slow host API (for example a network
/// bind check) never stalls backend responses and events behind it.
struct HostCallDispatcher {
    jobs: Option<mpsc::Sender<HostCallJob>>,
    call_tx: mpsc::Sender<BackendCall>,
}

impl HostCallDispatcher {
    fn new(call_tx: mpsc::Sender<BackendCall>) -> Self {
        let (jobs_tx, jobs_rx) = mpsc::unbounded::<HostCallJob>();
        let worker_call_tx = call_tx.clone();
        let jobs = std::thread::Builder::new()
            .name("tynd-bun-hostcall".into())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    answer_host_call(job, &worker_call_tx);
                }
            })
            .map_err(|error| tynd_host::tynd_error!("Failed to start host-call worker: {error}"))
            .ok()
            .map(|_| jobs_tx);
        Self { jobs, call_tx }
    }

    fn dispatch(&self, job: HostCallJob) {
        match &self.jobs {
            Some(jobs) => {
                if let Err(rejected) = jobs.send(job) {
                    answer_host_call(rejected.into_inner(), &self.call_tx);
                }
            },
            // No worker thread could be started: answer inline rather than drop the call.
            None => answer_host_call(job, &self.call_tx),
        }
    }
}

fn answer_host_call(job: HostCallJob, call_tx: &mpsc::Sender<BackendCall>) {
    let HostCallJob {
        id,
        api,
        method,
        args,
    } = job;
    let (ok, value) = match dispatch_backend_host_call(&api, &method, args) {
        Ok(value) => (true, value),
        Err(error) => (false, Value::String(error)),
    };
    let _ = call_tx.send(BackendCall::HostResult { id, ok, value });
}

pub(super) fn spawn_stderr_reader(stderr: ChildStderr) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("tynd-bun-stderr".into())
        .spawn(move || stderr_reader_loop(stderr))
        .map(|_| ())
}

fn stderr_reader_loop(stderr: ChildStderr) {
    let mut reader = BufReader::new(stderr);
    let mut line = Vec::with_capacity(1024);
    loop {
        match read_bounded_line(&mut reader, &mut line, MAX_LOG_LINE) {
            Ok(0) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(strip_trailing_eol(&line));
                if !text.is_empty() {
                    tynd_warn!("[bun:stderr] {text}");
                }
            },
            Err(error) => {
                tynd_host::tynd_error!("Bun stderr framing error: {error}");
                break;
            },
        }
    }
}

pub(super) fn spawn_stdout_reader(
    mut reader: BufReader<ChildStdout>,
    context: StdoutReaderContext,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("tynd-bun-stdout".into())
        .spawn(move || stdout_reader_loop(&mut reader, context))
        .map(|_| ())
}

fn stdout_reader_loop<R: BufRead>(reader: &mut R, context: StdoutReaderContext) {
    let config = match read_config(reader) {
        Ok(config) => config,
        Err(error) => {
            let _ = context.config_tx.send(Err(error));
            return;
        },
    };
    if context.config_tx.send(Ok(config)).is_err() {
        return;
    }

    let host_calls = HostCallDispatcher::new(context.call_tx.clone());
    json_reader_loop(reader, &context.event_tx, &host_calls);
    let controlled = context.shutdown_expected.load(Ordering::SeqCst);
    if !controlled {
        // EOF or a framing error ended the channel: the child is dead or can no
        // longer be understood. Reap/kill it (a child blocked on a full stdout
        // pipe would otherwise live until the host exits, and an exited one
        // stays a zombie) and drop stdin so outbound calls fail fast. A reload
        // sets `shutdown_expected` first, so a stale reader never touches the
        // replacement child.
        retire_dead_child(&context.child_slot, &context.stdin_slot);
    }
    if controlled || context.packaged_smoke {
        tynd_host::tynd_debug!("Bun stdout closed during controlled shutdown");
    } else {
        tynd_host::tynd_error!(
            "Bun subprocess exited unexpectedly — IPC channel is dead. The runtime crashed, was \
             OOM-killed, or terminated on its own. Restart the host to recover."
        );
        let _ = context.event_tx.send(BackendEvent::Error {
            message: "Backend runtime exited unexpectedly. Check host logs and restart."
                .to_string(),
        });
    }
}

/// Kill the child, then drop its stdin. The order matters: the stdin forwarder
/// holds the slot lock across a blocking `write_all`, and a child that stopped
/// draining its pipes never unblocks it. Killing first breaks the pipe so that
/// write fails (rejecting its calls with `BACKEND_DEAD`) and releases the lock.
fn retire_dead_child(child_slot: &Weak<ChildSlot>, stdin_slot: &Mutex<Option<ChildStdin>>) {
    if let Some(child_slot) = child_slot.upgrade() {
        child_slot.terminate();
    }
    *stdin_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// Forward a parsed `BunMsg` to the host event channel.
fn forward_bun_msg(msg: BunMsg, tx: &mpsc::Sender<BackendEvent>, host_calls: &HostCallDispatcher) {
    match msg {
        BunMsg::Return { id, ok, v, error } => {
            // Success: `v` carries the devalue payload. Error: `error` is a
            // plain string surfaced verbatim to the webview's resolve hook.
            let payload = if ok {
                BackendReturnPayload::Ok(v.unwrap_or_else(|| "null".into()))
            } else {
                BackendReturnPayload::Err(error.unwrap_or_else(|| "unknown error".into()))
            };
            let _ = tx.send(BackendEvent::Return { id, payload });
        },
        BunMsg::Yield { id, v } => {
            let _ = tx.send(BackendEvent::Yield { id, value: v });
        },
        BunMsg::Event { name, p, to } => {
            let _ = tx.send(BackendEvent::Emit {
                name,
                payload: p,
                to,
            });
        },
        BunMsg::EvalInFrontend { script, to } => {
            let _ = tx.send(BackendEvent::EvalInFrontend { script, to });
        },
        BunMsg::HostCall {
            id,
            api,
            method,
            args,
        } => {
            host_calls.dispatch(HostCallJob {
                id,
                api,
                method,
                args,
            });
        },
        BunMsg::Config { .. } => {
            // Stale config frame mid-stream — Bun re-emitted config after
            // handshake. Contract violation, but not worth crashing over.
        },
    }
}

/// Bun plugins are trusted application code, but they must not gain an
/// unbounded backdoor into every native host API. The backend bridge exposes
/// policy checks plus narrow bind validation; actual filesystem, network, or
/// process operations remain in Bun or go through the normal renderer ACL path.
fn dispatch_backend_host_call(api: &str, method: &str, args: Value) -> Result<Value, String> {
    if !backend_host_call_allowed(api, method) {
        return Err(format!(
            "backend host call '{api}.{method}' is not allowed; only policy and bind checks and \
             the startup trust handshake (app.setInfo, updater.configure) are exposed"
        ));
    }
    tynd_host::dispatch_api(api, method, args)
}

/// Allow-list for `host_call` frames from the Bun backend. `app.setInfo` (app
/// version) and `updater.configure` (trusted update endpoints and keys) are the
/// startup handshake: the host refuses both from a renderer, so the backend is
/// the only caller that can install them.
fn backend_host_call_allowed(api: &str, method: &str) -> bool {
    match api {
        "security" => matches!(
            method,
            "checkFs"
                | "checkHttp"
                | "checkProcess"
                | "checkAudioInput"
                | "checkAudioOutput"
                | "checkScreenCapture"
                | "checkWindow"
        ),
        "network-loopback" | "network-public" => method == "bind",
        "app" => method == "setInfo",
        "updater" => method == "configure",
        _ => false,
    }
}

/// `\n`-delimited JSON reader. Recycles `buf` across iterations to avoid the
/// per-line alloc that `BufRead::lines()` / `read_line` would do.
fn json_reader_loop<R: BufRead>(
    reader: &mut R,
    tx: &mpsc::Sender<BackendEvent>,
    host_calls: &HostCallDispatcher,
) {
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    loop {
        buf.clear();
        match read_bounded_json_line(reader, &mut buf) {
            Ok(0) => break,
            Err(error) => {
                tynd_host::tynd_error!("JSON framing error on Bun stdout: {error}");
                break;
            },
            Ok(_) => {
                let line_bytes = strip_trailing_eol(&buf);
                let Ok(l) = std::str::from_utf8(line_bytes) else {
                    continue;
                };
                let trimmed = l.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if !trimmed.starts_with('{') {
                    // Route through `tynd_info!` so the file-rotating
                    // tracing subscriber captures it; `eprintln!`
                    // disappears in packed Windows GUI mode (no console
                    // attached) and bypasses log rotation everywhere.
                    tynd_info!("[bun] {trimmed}");
                    continue;
                }
                match parse_bun_msg(trimmed) {
                    Ok(msg) => forward_bun_msg(msg, tx, host_calls),
                    Err(e) => tynd_warn!("Parse error ({e}): {trimmed}"),
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use std::{
        process::{Command, Stdio},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };

    use super::{
        backend_host_call_allowed, dispatch_backend_host_call, retire_dead_child, ChildSlot,
    };

    fn long_lived_command() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("ping");
            command.args(["-n", "60", "127.0.0.1"]);
            command
        }
        #[cfg(not(windows))]
        {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        }
    }

    #[test]
    fn retire_kills_the_child_before_waiting_for_a_stdin_writer() {
        let mut child = long_lived_command()
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a long-lived child");
        let stdin = child.stdin.take();
        let child_slot = Arc::new(ChildSlot(Mutex::new(Some(child))));
        let stdin_slot = Arc::new(Mutex::new(stdin));

        // Stand-in for the stdin forwarder: holds the stdin lock until the
        // child has been killed, as a blocked `write_all` would.
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let writer = {
            let child_slot = Arc::clone(&child_slot);
            let stdin_slot = Arc::clone(&stdin_slot);
            std::thread::spawn(move || {
                let _guard = stdin_slot.lock().unwrap();
                locked_tx.send(()).unwrap();
                let deadline = Instant::now() + Duration::from_secs(10);
                while child_slot.lock().is_some() {
                    if Instant::now() > deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                true
            })
        };
        locked_rx.recv().unwrap();

        retire_dead_child(&Arc::downgrade(&child_slot), &stdin_slot);

        assert!(
            writer.join().unwrap(),
            "child was not killed while stdin was locked"
        );
        assert!(stdin_slot.lock().unwrap().is_none());
    }

    #[test]
    fn allows_the_startup_trust_handshake_only() {
        assert!(backend_host_call_allowed("app", "setInfo"));
        assert!(backend_host_call_allowed("updater", "configure"));
        assert!(backend_host_call_allowed("security", "checkFs"));
        for method in [
            "checkAudioInput",
            "checkAudioOutput",
            "checkScreenCapture",
            "checkWindow",
        ] {
            assert!(backend_host_call_allowed("security", method), "{method}");
        }
        assert!(!backend_host_call_allowed(
            "security",
            "checkSensitiveCapability"
        ));
        assert!(backend_host_call_allowed("network-loopback", "bind"));
    }

    #[test]
    fn refuses_everything_else_on_app_and_updater() {
        for (api, method) in [
            ("app", "quit"),
            ("app", "getVersion"),
            ("app", "setinfo"),
            ("updater", "check"),
            ("updater", "install"),
            ("updater", "startPeriodicCheck"),
            ("updater", "downloadAndVerify"),
            ("security", "configure"),
            ("plugin", "x:configure"),
            ("fs", "readFile"),
        ] {
            assert!(!backend_host_call_allowed(api, method), "{api}.{method}");
        }
    }

    #[test]
    fn dispatch_rejects_a_disallowed_call_before_reaching_the_host() {
        let error = dispatch_backend_host_call("updater", "install", json!({})).unwrap_err();
        assert!(error.contains("updater.install"), "{error}");
    }
}
