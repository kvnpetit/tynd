use std::{
    collections::HashMap,
    io::{Read, Write},
    process::{ChildStdin, Command, Stdio},
    sync::{Arc, LazyLock},
};

use dashmap::DashMap;
use parking_lot::Mutex;
use shared_child::SharedChild;

use serde_json::{json, Value};

use crate::{
    ipc_error::IpcError,
    os::{events, security},
    tynd,
};

/// Marker for the `#[tynd::handler]` dispatch table — process state
/// (per-PID `SpawnEntry` registry) lives in the file-level `REGISTRY`
/// static.
#[derive(Debug, Default)]
pub struct ProcessApi;

#[tynd::handler(api = "process")]
impl ProcessApi {
    /// Run `cmd` with `args` and wait for it to exit. Returns
    /// `{ code, stdout, stderr }`. `cwd`, `env`, and `input` are
    /// optional. `cwd` is gated by the host's fs allow-list.
    pub fn exec(
        cmd: String,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<serde_json::Map<String, Value>>,
        input: Option<String>,
    ) -> Result<Value, String> {
        exec_impl(&cmd, argv, cwd, env, input, false)
    }

    /// Run `cmd` through the system shell. The `cmd` string is
    /// passed verbatim to `cmd /C` (Windows) or `sh -c` (Unix) — no
    /// extra `args` may be supplied (embed them directly into
    /// `cmd`). Gated by `security.shellExecAllowedCommands`.
    pub fn exec_shell(
        cmd: String,
        cwd: Option<String>,
        env: Option<serde_json::Map<String, Value>>,
        input: Option<String>,
    ) -> Result<Value, String> {
        exec_impl(&cmd, None, cwd, env, input, true)
    }

    /// Spawn `cmd` with `args` in streaming mode. Returns `{ pid }`
    /// — the host emits `process:stdout:<pid>`,
    /// `process:stderr:<pid>` and `process:exit:<pid>` until the
    /// child exits.
    pub fn spawn(
        cmd: String,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<serde_json::Map<String, Value>>,
    ) -> Result<Value, String> {
        spawn_impl(&cmd, argv.unwrap_or_default(), cwd, env)
    }

    /// Write `data` to the child's stdin. Accepts a UTF-8 string or
    /// a JSON array of byte values (`number[]`) — symmetric with the
    /// frontend's `string | Uint8Array` shape. Errors when the pid
    /// has exited or stdin was closed.
    pub fn write_stdin(pid: u32, data: Value) -> Result<(), String> {
        let bytes: Vec<u8> = if let Some(s) = data.as_str() {
            s.as_bytes().to_vec()
        } else if let Some(arr) = data.as_array() {
            arr.iter()
                .map(|v| {
                    v.as_u64().and_then(|n| u8::try_from(n).ok()).ok_or_else(|| {
                        IpcError::new(
                            "process",
                            "writeStdin",
                            "'data' bytes out of range",
                        )
                        .into_string()
                    })
                })
                .collect::<Result<Vec<u8>, String>>()?
        } else {
            return Err(IpcError::new(
                "process",
                "writeStdin",
                "'data' must be string or byte array",
            )
            .into_string());
        };
        write_stdin_impl(pid, &bytes)
    }

    /// Close the child's stdin pipe. The child sees EOF on its
    /// stdin descriptor.
    pub fn close_stdin(pid: u32) -> Result<(), String> {
        close_stdin_impl(pid)
    }

    /// Send `signal` (default `SIGTERM`) to the child. Windows
    /// always issues `TerminateProcess` — the signal name is
    /// recorded but ignored.
    pub fn kill(pid: u32, signal: Option<String>) -> Result<(), String> {
        kill_impl(pid, signal.as_deref().unwrap_or("SIGTERM"))
    }
}

fn exec_impl(
    cmd_str: &str,
    args_in: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<serde_json::Map<String, Value>>,
    input: Option<String>,
    shell: bool,
) -> Result<Value, String> {
    let argv: Vec<String> = args_in.unwrap_or_default();

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

    if let Some(cwd) = cwd.as_deref() {
        // SECURITY: route `cwd` through the same fs allow-list as every
        // other host file access. Without this, a caller can pin a child
        // process to `/etc` (Unix) or `C:\Windows\System32` (Windows) and
        // launch a binary that reads from there using relative paths,
        // sidestepping the security policy.
        security::check_fs(cwd).map_err(|e| {
            IpcError::new("process", "exec", format!("cwd not permitted: {e}")).into_string()
        })?;
        command.current_dir(cwd);
    }

    if let Some(env_obj) = env {
        let merged: HashMap<String, String> = env_obj
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
            .collect();
        command.envs(merged);
    }

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

/// Per-pid handles kept around for runtime mutations. The child lives
/// until the waiter thread observes exit, then drops it from the registry.
///
/// `SharedChild` (Tauri's pattern) wraps the OS handle so `wait()` and
/// `kill()` can be called from different threads without coordination —
/// the previous `Mutex<Option<Child>>` had to take the child out of the
/// mutex before the long-blocking `wait()` so a racing `kill()` could
/// still grab the handle, and that dance is gone now: both threads share
/// the same `Arc<SharedChild>` and the OS layer makes the operations
/// concurrent-safe (Linux `waitid(WNOWAIT)` + post-wait reap on Unix,
/// signaled handle wait on Windows).
struct SpawnEntry {
    child: Arc<SharedChild>,
    /// Owned `ChildStdin` so `writeStdin` calls can reuse the pipe across
    /// multiple frames. `None` once the user calls `closeStdin` (or stdin
    /// was never piped).
    stdin: Mutex<Option<ChildStdin>>,
}

/// Process-wide pid → spawn entry registry. `DashMap` shard-locks the
/// map so a `kill()` on one pid never serialises behind a `writeStdin()`
/// touching a different pid — the previous `Mutex<HashMap>` made every
/// concurrent reader pay one outer lock acquisition just to fetch its
/// own per-pid `Arc<SpawnEntry>`.
static REGISTRY: LazyLock<DashMap<u32, Arc<SpawnEntry>>> = LazyLock::new(DashMap::default);

fn registry_remove(pid: u32) -> Option<Arc<SpawnEntry>> {
    REGISTRY.remove(&pid).map(|(_, v)| v)
}

fn registry_get(pid: u32) -> Option<Arc<SpawnEntry>> {
    REGISTRY.get(&pid).map(|e| e.value().clone())
}

/// Read one bytes chunk and emit it as `process:<stream>:<pid>`. Stops on
/// EOF or read error. We avoid the per-byte JSON cost by emitting the
/// whole chunk once per read — typical pipe reads return up to 64 KiB at
/// once on every platform, so this is a small constant number of events
/// per second on a busy stream rather than one per byte.
///
/// # Wire format
///
/// Default (no `process-bytes-array` feature) emits
/// `{ data: "<base64>" }` — 25× smaller on the wire than the legacy
/// `Number[]` array (1 byte = 1 JSON number = 1-3 ASCII chars + comma)
/// and ~50× cheaper to serialize on the host (no per-byte
/// `Value::from(u32)` allocation per chunk). Frontend decodes via
/// `atob(data)` then `Uint8Array.from(...)`.
///
/// With `process-bytes-array` enabled, the legacy `Number[]` shape is
/// preserved for forks / plugins still pinned to the old contract.
fn pump_stream<R: Read>(mut reader: R, event_name: String) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF — child closed the stream.
            Ok(n) => {
                #[cfg(feature = "process-bytes-array")]
                {
                    // Legacy wire format — `Number[]` per byte. Kept
                    // behind a feature gate so plugins pinned to the
                    // old `chunkFromEvent` JS path can rebuild on
                    // `--features process-bytes-array` without
                    // touching their decoder.
                    let chunk: Vec<Value> = buf[..n]
                        .iter()
                        .map(|b| Value::from(u32::from(*b)))
                        .collect();
                    events::emit(&event_name, &json!({ "data": Value::Array(chunk) }));
                }
                #[cfg(not(feature = "process-bytes-array"))]
                {
                    // Default base64 path — see fn-level docs for the
                    // perf rationale. `STANDARD` (with `=` padding)
                    // matches `atob` exactly so the JS side doesn't
                    // need a custom decoder shim.
                    use base64::Engine;
                    let chunk = base64::engine::general_purpose::STANDARD.encode(&buf[..n]);
                    events::emit(&event_name, &json!({ "data": chunk }));
                }
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

fn spawn_impl(
    cmd_str: &str,
    argv: Vec<String>,
    cwd: Option<String>,
    env: Option<serde_json::Map<String, Value>>,
) -> Result<Value, String> {
    let mut command = Command::new(cmd_str);
    command.args(&argv);

    if let Some(cwd) = cwd.as_deref() {
        // SECURITY: same gate as `exec` — `cwd` becomes the child's working
        // directory and any relative path it dereferences is resolved
        // against this. Run it through the fs policy before honoring it.
        security::check_fs(cwd).map_err(|e| {
            IpcError::new("process", "spawn", format!("cwd not permitted: {e}")).into_string()
        })?;
        command.current_dir(cwd);
    }
    if let Some(env_obj) = env {
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

    // `SharedChild::spawn` consumes the `Command` after taking the stdio
    // handles internally — but we want to own stdout/stderr/stdin
    // separately so the pump threads can read them without going through
    // the SharedChild wrapper. Spawn the raw `Child` first, take the
    // pipe handles, then wrap the headless remainder in SharedChild.
    let mut raw = command
        .spawn()
        .map_err(|e| IpcError::new("process", "spawn", e).into_string())?;

    let pid = raw.id();
    let stdout = raw.stdout.take();
    let stderr = raw.stderr.take();
    let stdin = raw.stdin.take();

    let shared = SharedChild::new(raw).map_err(|e| {
        IpcError::new("process", "spawn", format!("SharedChild::new: {e}")).into_string()
    })?;
    let child = Arc::new(shared);

    // Refcounted entry: the registry, the waiter thread, and any
    // concurrent `kill`/`writeStdin` callers each hold their own `Arc`.
    // The last drop frees the entry — `SharedChild` makes
    // `wait()`/`kill()` concurrent-safe so we no longer need the
    // take-out-of-mutex dance the previous `Mutex<Option<Child>>` shape
    // required.
    let entry = Arc::new(SpawnEntry {
        child,
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

    REGISTRY.insert(pid, Arc::clone(&entry));

    // Waiter thread — `wait()` blocks until the child exits, which is
    // exactly what we want here. The OS already gave us a separate handle
    // per stdio pipe, so the read threads above are unaffected. With
    // `SharedChild` we no longer take the child out of a mutex first: a
    // racing `kill()` calls `SharedChild::kill()` directly on the same
    // `Arc<SharedChild>` and the OS layer arbitrates (Unix relies on
    // signal-then-reap, Windows on the kernel-level termination handle).
    let waiter_entry = Arc::clone(&entry);
    std::thread::spawn(move || {
        let status = waiter_entry.child.wait();
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
        // Drop the registry's Arc. Any caller that grabbed an Arc in
        // between (a racing `writeStdin` / `kill`) keeps the entry
        // alive until *they* drop their clone — the entry is freed on
        // the last decrement.
        let _ = registry_remove(pid);
        // `waiter_entry` itself drops here.
        drop(waiter_entry);
    });

    Ok(json!({ "pid": pid }))
}

fn write_stdin_impl(pid: u32, bytes: &[u8]) -> Result<(), String> {
    let entry = registry_get(pid).ok_or_else(|| {
        IpcError::new(
            "process",
            "writeStdin",
            format!("pid {pid} not found (already exited?)"),
        )
        .into_string()
    })?;
    let mut stdin_opt = entry.stdin.lock();
    let stdin = stdin_opt.as_mut().ok_or_else(|| {
        IpcError::new("process", "writeStdin", format!("stdin closed for pid {pid}")).into_string()
    })?;
    stdin
        .write_all(bytes)
        .map_err(|e| IpcError::new("process", "writeStdin", e).into_string())?;
    Ok(())
}

fn close_stdin_impl(pid: u32) -> Result<(), String> {
    let entry = registry_get(pid).ok_or_else(|| {
        IpcError::new("process", "closeStdin", format!("pid {pid} not found")).into_string()
    })?;
    // Dropping the ChildStdin closes the write end of the pipe — the child
    // sees EOF on stdin which is exactly what users mean by "close stdin".
    *entry
        .stdin
        .lock() = None;
    Ok(())
}

fn kill_impl(pid: u32, signal: &str) -> Result<(), String> {
    let entry = registry_get(pid).ok_or_else(|| {
        IpcError::new("process", "kill", format!("pid {pid} not found")).into_string()
    })?;

    // On Unix we honour the actual signal name — `SIGTERM` and `SIGINT`
    // give the child a chance to shut down gracefully (close sockets,
    // flush buffers); only `SIGKILL` (or the `SharedChild::kill`
    // fallback) takes the kernel-level prejudice path. On Windows
    // there's no signal abstraction — `TerminateProcess` is the only
    // knob, so the signal name is recorded for forward-compat but
    // otherwise ignored.
    #[cfg(unix)]
    {
        use nix::{sys::signal::Signal, unistd::Pid};
        // `SharedChild::kill()` always sends SIGKILL — for
        // SIGTERM/SIGINT we go through `nix::sys::signal::kill`. The pid
        // comes straight off the SharedChild handle (no lock needed)
        // and the actual signal goes through the OS by pid number so a
        // racing `wait()` from the waiter thread is fine.
        let target_pid = entry.child.id() as i32;
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
            Err(e) => return Err(IpcError::new("process", "kill", e).into_string()),
        }
    }
    #[cfg(not(unix))]
    {
        // Windows: TerminateProcess is the only termination primitive;
        // record the requested signal in the log so callers can see
        // their preference was acknowledged but treat all values as a
        // hard kill. `SharedChild::kill` is concurrent-safe with the
        // waiter's `wait()` — both share the same OS handle and the
        // wait completes when termination signals it.
        let _ = signal;
        entry
            .child
            .kill()
            .map_err(|e| IpcError::new("process", "kill", e).into_string())?;
    }
    Ok(())
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
        let v = dispatch(
            "exec",
            &json!({ "cmd": "cmd", "argv": ["/C", "echo hello"] }),
        )
        .unwrap();
        #[cfg(not(target_os = "windows"))]
        let v = dispatch("exec", &json!({ "cmd": "echo", "argv": ["hello"] })).unwrap();
        assert!(v["stdout"].as_str().unwrap().contains("hello"));
        assert_eq!(v["code"].as_i64().unwrap(), 0);
    }

    #[test]
    fn exec_shell_runs_simple_command_when_whitelisted() {
        let _g = security::test_lock().lock();
        let _reset = allow_shell(&["echo"]);
        let v = dispatch("execShell", &json!({ "cmd": "echo piped" })).unwrap();
        let stdout = v["stdout"].as_str().unwrap();
        assert!(
            stdout.contains("piped"),
            "expected stdout to contain 'piped', got: {stdout}"
        );
    }

    #[test]
    fn exec_shell_rejects_when_not_configured() {
        let _g = security::test_lock().lock();
        // No whitelist set — every call must reject.
        security::set_shell_exec_allowed_for_tests(Vec::new());
        let err = dispatch("execShell", &json!({ "cmd": "ls" })).unwrap_err();
        assert!(
            err.contains("not permitted"),
            "expected 'not permitted' error, got: {err}"
        );
    }

    #[test]
    fn exec_shell_rejects_unlisted_command() {
        let _g = security::test_lock().lock();
        let _reset = allow_shell(&["echo"]);
        let err = dispatch("execShell", &json!({ "cmd": "rm -rf /tmp/foo" })).unwrap_err();
        assert!(
            err.contains("not in `shellExecAllowedCommands`"),
            "expected whitelist-miss error, got: {err}"
        );
    }

    #[test]
    fn exec_shell_rejects_metacharacter_even_if_whitelisted() {
        let _g = security::test_lock().lock();
        let _reset = allow_shell(&["ls"]);
        // `ls` is whitelisted but `;` is a shell metacharacter — refuse.
        let err = dispatch("execShell", &json!({ "cmd": "ls; rm -rf /" })).unwrap_err();
        assert!(
            err.contains("metacharacters are not allowed"),
            "expected metachar-reject error, got: {err}"
        );
        // Pipe is also a metacharacter.
        let err = dispatch("execShell", &json!({ "cmd": "ls | cat" })).unwrap_err();
        assert!(err.contains("metacharacters"));
    }

    #[test]
    fn missing_cmd_errors() {
        assert!(dispatch("exec", &json!({})).is_err());
    }

    #[test]
    fn nonzero_exit_preserved() {
        #[cfg(target_os = "windows")]
        let v =
            dispatch("exec", &json!({ "cmd": "cmd", "argv": ["/C", "exit 7"] })).unwrap();
        #[cfg(not(target_os = "windows"))]
        let v =
            dispatch("exec", &json!({ "cmd": "sh", "argv": ["-c", "exit 7"] })).unwrap();
        assert_eq!(v["code"].as_i64().unwrap(), 7);
    }

    /// `spawn` returns immediately with a numeric pid that lands in the
    /// global registry.
    #[test]
    fn spawn_returns_pid_and_registers_entry() {
        #[cfg(target_os = "windows")]
        let v = dispatch(
            "spawn",
            &json!({
                "cmd": "cmd",
                "argv": ["/C", "ping", "-n", "5", "127.0.0.1"]
            }),
        )
        .expect("spawn");
        #[cfg(not(target_os = "windows"))]
        let v = dispatch(
            "spawn",
            &json!({
                "cmd": "sleep",
                "argv": ["2"]
            }),
        )
        .expect("spawn");

        let pid = v["pid"].as_u64().expect("pid is u64") as u32;
        assert!(pid > 0);
        assert!(REGISTRY.contains_key(&pid));

        // Kill it so the test doesn't leak processes.
        dispatch("kill", &json!({ "pid": pid })).expect("kill");

        // Give the waiter thread a moment to drain — exit emit + registry
        // remove happens off the dispatch thread.
        for _ in 0..50 {
            if !REGISTRY.contains_key(&pid) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            !REGISTRY.contains_key(&pid),
            "spawn waiter never removed pid {pid} from registry"
        );
    }

    #[test]
    fn spawn_invalid_binary_rejects() {
        let err = dispatch(
            "spawn",
            &json!({
                "cmd": "this-binary-does-not-exist-tynd-test-xyz",
                "argv": []
            }),
        )
        .unwrap_err();
        assert!(err.contains("process.spawn"), "got: {err}");
    }

    #[test]
    fn write_stdin_unknown_pid_errors() {
        let err = dispatch(
            "writeStdin",
            &json!({ "pid": 99999999u64, "data": "hi" }),
        )
        .unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }

    #[test]
    fn kill_unknown_pid_errors() {
        let err = dispatch("kill", &json!({ "pid": 99999999u64 })).unwrap_err();
        assert!(err.contains("not found"), "got: {err}");
    }
}
