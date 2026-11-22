use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::Value;

use crate::ipc::error::IpcError;
use crate::{api::HostApi, security::policy, tynd};

mod encoding;
mod exec;
mod registry;
mod spawn;

use exec::{exec_impl, ExecRequest};
use spawn::{activate_impl, close_stdin_impl, kill_impl, spawn_impl, write_stdin_impl};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// The document of webview `label` is gone (commit or close): its running
/// processes are re-owned to the backend instead of killed, so their output
/// stops going to whatever the label shows next.
pub(crate) fn release_webview(label: &str) {
    registry::release_owner(label);
}

/// Stop accepting new children, terminate every registered child, then wait
/// against one shared deadline. Killing every child before waiting keeps the
/// bound global instead of multiplying it by the registry size.
pub(crate) fn shutdown_all() {
    let entries = registry::begin_shutdown();
    if entries.is_empty() {
        return;
    }

    for (pid, entry) in &entries {
        // Closing stdin first lets cooperative children observe EOF, while the
        // following hard kill guarantees cleanup remains bounded.
        *entry.stdin.lock() = None;
        entry.activate();
        if let Err(error) = entry.child.kill() {
            tracing::warn!(
                target: "tynd::cleanup",
                pid,
                %error,
                "failed to terminate child process during shutdown"
            );
        }
    }

    let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
    for (pid, entry) in &entries {
        match entry.child.wait_deadline(deadline) {
            Ok(Some(_)) => {},
            Ok(None) => tracing::warn!(
                target: "tynd::cleanup",
                pid,
                "child process did not exit before shutdown deadline"
            ),
            Err(error) => tracing::warn!(
                target: "tynd::cleanup",
                pid,
                %error,
                "failed to wait for child process during shutdown"
            ),
        }
    }

    // Waiter threads own registry removal and spawn-slot release. Give them
    // the remainder of the same deadline; never add a second timeout window.
    while !registry::is_drained() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }

    let remaining = registry::remaining_pids();
    if !remaining.is_empty() {
        tracing::warn!(
            target: "tynd::cleanup",
            ?remaining,
            "child process registry was not fully drained before shutdown deadline"
        );
    }
}

/// Dispatch marker — actual per-PID state lives in `REGISTRY` below.
#[derive(Debug, Default)]
pub(crate) struct ProcessApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown_all),
    release_webview: Some(release_webview),
    ..HostApi::new("process", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "process")]
impl ProcessApi {
    /// Run `cmd` with `args` and wait for it to exit. Returns
    /// `{ code, stdout, stderr }`. `cwd`, `env`, and `input` are
    /// optional. `cwd` is gated by the host's fs allow-list.
    ///
    /// `inputEncoding` controls how `input` becomes the byte stream
    /// piped to the child's stdin: `"utf8"` (default) writes the raw
    /// UTF-8 bytes of the string; `"base64"` decodes the string as
    /// standard-alphabet base64 first.
    ///
    /// `outputEncoding` controls how `stdout` and `stderr` are
    /// returned to the caller: `"utf8"` (default) returns
    /// `String::from_utf8_lossy` (replacement chars on invalid
    /// UTF-8); `"base64"` returns the raw bytes as standard-alphabet
    /// base64 — the only safe choice for binary output.
    // Handler parameters mirror stable renderer payload keys; grouping them would break wire ABI.
    #[allow(clippy::too_many_arguments)]
    pub fn exec(
        cmd: String,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<serde_json::Map<String, Value>>,
        input: Option<String>,
        input_encoding: Option<String>,
        output_encoding: Option<String>,
        timeout_ms: Option<u64>,
        max_output_bytes: Option<u64>,
    ) -> Result<Value, String> {
        // SECURITY: gate the binary against `processAllowedBinaries`.
        // Closed-by-default — unconfigured policy rejects every call.
        policy::check_process(&cmd)?;
        exec_impl(ExecRequest {
            command: cmd,
            argv: argv.unwrap_or_default(),
            cwd,
            env,
            input,
            input_encoding,
            output_encoding,
            timeout_ms,
            max_output_bytes,
            shell: false,
        })
    }

    /// Run `cmd` through the system shell. The `cmd` string is
    /// passed verbatim to `cmd /C` (Windows) or `sh -c` (Unix) — no
    /// extra `args` may be supplied (embed them directly into
    /// `cmd`). Gated by `security.shellExecAllowedCommands`.
    ///
    /// `inputEncoding` and `outputEncoding` semantics match `exec`.
    pub fn exec_shell(
        cmd: String,
        cwd: Option<String>,
        env: Option<serde_json::Map<String, Value>>,
        input: Option<String>,
        input_encoding: Option<String>,
        output_encoding: Option<String>,
        timeout_ms: Option<u64>,
        max_output_bytes: Option<u64>,
    ) -> Result<Value, String> {
        exec_impl(ExecRequest {
            command: cmd,
            argv: Vec::new(),
            cwd,
            env,
            input,
            input_encoding,
            output_encoding,
            timeout_ms,
            max_output_bytes,
            shell: true,
        })
    }

    /// Reserve a streaming child and return `{ pid }`. Reader and waiter
    /// workers remain gated until `activate(pid)`, allowing the renderer to
    /// install stdout, stderr, EOF and exit listeners without an event race.
    pub fn spawn(
        cmd: String,
        argv: Option<Vec<String>>,
        cwd: Option<String>,
        env: Option<serde_json::Map<String, Value>>,
    ) -> Result<Value, String> {
        // SECURITY: gate against `processAllowedBinaries` before spawning
        // any long-running child. Closed-by-default — unconfigured policy
        // rejects every call.
        policy::check_process(&cmd)?;
        spawn_impl(&cmd, argv.unwrap_or_default(), cwd, env)
    }

    /// Release a process reservation after renderer listeners are installed.
    /// Idempotent for repeated activation of a live process.
    pub fn activate(pid: u32) -> Result<(), String> {
        activate_impl(pid)
    }

    /// Write `data` to the child's stdin. `data` is a base64 string of the
    /// bytes to write: UTF-8 strings on the JS side are `TextEncoder`-encoded,
    /// then base64'd, before reaching here so a single decode produces the
    /// byte stream.
    ///
    /// Errors when the pid has exited or stdin was closed.
    pub fn write_stdin(pid: u32, data: Value) -> Result<(), String> {
        let bytes: Vec<u8> = if let Some(s) = data.as_str() {
            // Base64 decode is the canonical wire format. A malformed
            // string surfaces as a typed error (e.g. caller forgot
            // `bytesToBase64`) rather than silently writing garbage.
            base64::engine::general_purpose::STANDARD
                .decode(s.as_bytes())
                .map_err(|e| {
                    proc_err(
                        "writeStdin",
                        format!("'data' string must be base64-encoded: {e}"),
                    )
                })?
        } else {
            return Err(proc_err("writeStdin", "'data' must be a base64 string"));
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

/// Build the `process.<method>: <msg>` IPC error string.
#[cold]
fn proc_err(method: &str, msg: impl std::fmt::Display) -> String {
    IpcError::new("process", method, msg).into_string()
}

pub(super) fn canonicalize_cwd(method: &str, cwd: &str) -> Result<std::path::PathBuf, String> {
    policy::authorize_fs_path(cwd).map_err(|e| proc_err(method, format!("cwd not permitted: {e}")))
}

pub(super) fn prepare_command(
    command: &mut std::process::Command,
    method: &str,
    cwd: Option<&str>,
    env: Option<&serde_json::Map<String, Value>>,
) -> Result<(), String> {
    // Suppress the black `cmd` flash that GUI-subsystem hosts otherwise emit
    // when spawning a console child. Console output is still piped.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    if let Some(cwd) = cwd {
        command.current_dir(canonicalize_cwd(method, cwd)?);
    }

    if let Some(env_obj) = env {
        if let Some(bad) = env_obj
            .keys()
            .find(|key| tynd_security_policy::env_key_denied(key))
        {
            return Err(proc_err(
                method,
                format!("env: '{bad}' is on the loader-hijack denylist; refusing"),
            ));
        }
        command.envs(
            env_obj
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key, value))),
        );
    }

    Ok(())
}
