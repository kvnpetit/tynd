#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use crossbeam_channel as mpsc;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tynd_host::{
    runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent, MenuItemDef, TrayConfig},
    tynd_log,
    window::WindowConfig,
};

/// Outbound wire frame to Bun's stdin. Single typed enum so all four
/// shapes share the same builder + serializer call site instead of
/// four near-identical `serde_json::json!()` blobs in the forwarder
/// loop. `serde(tag = "type")` reproduces the exact wire contract the
/// JS side expects (`{ "type": "...", ... }`).
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum WireMsg<'a> {
    #[serde(rename = "tynd:ready")]
    Ready,
    #[serde(rename = "tynd:close")]
    Close,
    #[serde(rename = "call")]
    Call {
        id: &'a str,
        #[serde(rename = "fn")]
        fn_name: &'a str,
        args: &'a [Value],
    },
    #[serde(rename = "cancel")]
    Cancel { id: &'a str },
    #[serde(rename = "ack")]
    Ack { id: &'a str, n: u32 },
}

/// Strip the trailing `\n` / `\r\n` `read_until(b'\n', ...)` left in the
/// reusable buffer. The line-byte view is borrowed from `buf` and lives
/// only for the current iteration's work.
#[inline]
fn strip_trailing_eol(buf: &[u8]) -> &[u8] {
    let mut end = buf.len();
    if end > 0 && buf[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && buf[end - 1] == b'\r' {
        end -= 1;
    }
    &buf[..end]
}

/// JSON decode helper for Bun stdout frames. Routes through the SIMD
/// path under the `fast-json` feature on supported architectures and
/// falls back to `serde_json::from_str` otherwise. Centralizing the
/// `cfg` mess here keeps the reader loop readable.
fn parse_bun_msg(s: &str) -> Result<BunMsg, String> {
    #[cfg(all(feature = "fast-json", target_arch = "x86_64"))]
    {
        return sonic_rs::from_str::<BunMsg>(s).map_err(|e| e.to_string());
    }
    #[cfg(all(feature = "fast-json", target_arch = "aarch64"))]
    {
        // simd-json mutates the input buffer in-place; copy then parse.
        let mut buf = s.as_bytes().to_vec();
        return simd_json::serde::from_slice::<BunMsg>(&mut buf).map_err(|e| e.to_string());
    }
    #[cfg(any(
        not(feature = "fast-json"),
        not(any(target_arch = "x86_64", target_arch = "aarch64"))
    ))]
    {
        serde_json::from_str::<BunMsg>(s).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum BunMsg {
    #[serde(rename = "tynd:config")]
    Config {
        window: Option<WindowConfig>,
        #[serde(rename = "devUrl")]
        dev_url: Option<String>,
        #[serde(rename = "frontendDir")]
        frontend_dir: Option<String>,
        #[serde(default)]
        menu: Vec<MenuItemDef>,
        tray: Option<TrayConfig>,
        #[serde(default, rename = "quitOnLastWindowClosed")]
        quit_on_last_window_closed: bool,
    },
    #[serde(rename = "return")]
    Return {
        id: String,
        ok: bool,
        /// Devalue-encoded payload string. Bun emits this for every successful
        /// return; the webview shim devalue-parses it. On error, only `error`
        /// is populated.
        #[serde(default)]
        v: Option<String>,
        error: Option<String>,
    },
    #[serde(rename = "yield")]
    Yield { id: String, v: String },
    #[serde(rename = "event")]
    Event {
        name: String,
        /// Devalue-encoded payload string for user emits. Routed verbatim
        /// to the webview shim which devalue-parses. The framework's
        /// `evalInFrontend` uses the separate `EvalInFrontend` variant
        /// because Rust needs to read `script` natively.
        p: String,
        #[serde(default)]
        to: Option<String>,
    },
    #[serde(rename = "evalInFrontend")]
    EvalInFrontend {
        script: String,
        #[serde(default)]
        to: Option<String>,
    },
}

/// Dev-mode handle: swap the Bun subprocess in place while the host and WebView stay alive.
#[derive(Clone)]
pub(crate) struct ReloadHandle {
    entry_path: String,
    stdin_slot: Arc<Mutex<Option<ChildStdin>>>,
    child_slot: Arc<Mutex<Option<Child>>>,
    event_tx: mpsc::Sender<BackendEvent>,
    /// Set to `true` for the duration of a controlled reload kill so the
    /// reader thread can tell an expected stdout-close apart from a real
    /// Bun crash. Shared between the reload trigger and the reader.
    reload_in_progress: Arc<AtomicBool>,
}

impl ReloadHandle {
    /// Frontend-only reload. Fires the webview reload via the shared bus
    /// without respawning Bun — used when only static frontend files
    /// changed on disk.
    pub(crate) fn reload_frontend_only(&self) {
        let _ = self.event_tx.send(BackendEvent::Reload);
    }

    pub(crate) fn reload(&self) {
        // Mark the upcoming stdout close as expected so the reader thread
        // doesn't surface it as a Bun crash.
        self.reload_in_progress.store(true, Ordering::SeqCst);

        // Killing the child closes stdout which unblocks the reader thread.
        if let Some(mut child) = self.child_slot.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        *self.stdin_slot.lock().unwrap() = None;

        match spawn_bun(
            &self.entry_path,
            &self.event_tx,
            self.reload_in_progress.clone(),
        ) {
            Ok((child, stdin, _config)) => {
                *self.child_slot.lock().unwrap() = Some(child);
                *self.stdin_slot.lock().unwrap() = Some(stdin);
                // The new reader is up; clear the flag so the *next* close
                // is treated as unexpected unless reload() flips it again.
                self.reload_in_progress.store(false, Ordering::SeqCst);
                let _ = self.event_tx.send(BackendEvent::Reload);
            },
            Err(e) => {
                self.reload_in_progress.store(false, Ordering::SeqCst);
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: format!("Failed to reload backend: {e}"),
                });
            },
        }
    }
}

/// Spawn a Bun subprocess, wait for the initial config, and return the bridge
/// plus a reload handle. When launched by a packed binary, TYND_BUN_PATH points
/// to the embedded runtime instead of the system bun.
pub(crate) fn start(entry_path: &str) -> (BackendBridge, ReloadHandle) {
    let (call_tx, call_rx) = mpsc::unbounded::<BackendCall>();
    let (event_tx, event_rx) = mpsc::unbounded::<BackendEvent>();
    let reload_in_progress = Arc::new(AtomicBool::new(false));

    let (child, stdin, config) = match spawn_bun(entry_path, &event_tx, reload_in_progress.clone())
    {
        Ok(t) => t,
        Err(e) => {
            tynd_log!("{e}");
            std::process::exit(1);
        },
    };

    let stdin_slot: Arc<Mutex<Option<ChildStdin>>> = Arc::new(Mutex::new(Some(stdin)));
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));

    // Forwarder: call_rx -> current Bun stdin (under mutex so reload can swap it).
    //
    // B18: coalesces multiple queued BackendCall frames into a single
    // `write_all` so a burst (e.g. an `ack` storm + a parallel `cancel`)
    // is one syscall instead of N. After each blocking `recv`, drain
    // anything already buffered via `try_recv` and flush the whole batch
    // at once. -90% syscalls on bursty stdin write paths.
    {
        let stdin_slot = stdin_slot.clone();
        std::thread::spawn(move || {
            let mut burst: Vec<u8> = Vec::with_capacity(8 * 1024);
            while let Ok(call) = call_rx.recv() {
                burst.clear();
                push_call_line(&mut burst, call);
                while let Ok(extra) = call_rx.try_recv() {
                    push_call_line(&mut burst, extra);
                    // Cap burst so an unbounded stdin storm doesn't
                    // pin > 1 MiB in this thread; the next iteration
                    // drains the rest.
                    if burst.len() >= 1024 * 1024 {
                        break;
                    }
                }
                if burst.is_empty() {
                    continue;
                }
                if let Some(stdin) = stdin_slot.lock().unwrap().as_mut() {
                    let _ = stdin.write_all(&burst);
                }
            }
        });
    }

    let bridge = BackendBridge {
        config,
        call_tx,
        event_rx,
    };
    let reload = ReloadHandle {
        entry_path: entry_path.to_string(),
        stdin_slot,
        child_slot,
        event_tx,
        reload_in_progress,
    };
    (bridge, reload)
}

/// Append the wire-encoded `\n`-terminated line for one BackendCall to
/// the coalescing buffer. Used by the stdin forwarder (B18) to batch
/// multiple frames into a single `write_all` syscall.
fn push_call_line(out: &mut Vec<u8>, call: BackendCall) {
    let bytes = match call {
        BackendCall::Raw(mut s) => {
            s.push('\n');
            s.into_bytes()
        },
        BackendCall::Typed { id, fn_name, args } => {
            let msg = match fn_name.as_str() {
                "__tynd_on_ready__" => WireMsg::Ready,
                "__tynd_on_close__" => WireMsg::Close,
                _ => WireMsg::Call {
                    id: &id,
                    fn_name: &fn_name,
                    args: &args,
                },
            };
            match serde_json::to_string(&msg) {
                Ok(mut s) => {
                    s.push('\n');
                    s.into_bytes()
                },
                Err(e) => {
                    tynd_host::tynd_error!("failed to serialize BackendCall: {e}");
                    return;
                },
            }
        },
        BackendCall::Cancel { id } => match serde_json::to_string(&WireMsg::Cancel { id: &id }) {
            Ok(mut s) => {
                s.push('\n');
                s.into_bytes()
            },
            Err(_) => return,
        },
        BackendCall::Ack { id, n } => match serde_json::to_string(&WireMsg::Ack { id: &id, n }) {
            Ok(mut s) => {
                s.push('\n');
                s.into_bytes()
            },
            Err(_) => return,
        },
    };
    out.extend_from_slice(&bytes);
}

/// Spawn a Bun subprocess, wait for `tynd:config`, spawn the stdout reader thread,
/// and return the child handle + stdin + parsed config.
fn spawn_bun(
    entry_path: &str,
    event_tx: &mpsc::Sender<BackendEvent>,
    reload_in_progress: Arc<AtomicBool>,
) -> Result<(Child, ChildStdin, BackendConfig), String> {
    let bun_bin = std::env::var("TYND_BUN_PATH").unwrap_or_else(|_| "bun".into());

    let mut cmd = Command::new(&bun_bin);
    cmd.arg("run")
        .arg(entry_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if cfg!(debug_assertions) {
            Stdio::inherit()
        } else {
            Stdio::null()
        });

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().map_err(|e| {
        let mut msg = format!("Failed to start runtime: {e}");
        if std::env::var("TYND_BUN_PATH").is_err() {
            msg.push_str("\nEnsure runtime is installed and in PATH");
        }
        msg
    })?;

    let bun_stdin = child.stdin.take().expect("Bun stdin not piped");
    let bun_stdout = child.stdout.take().expect("Bun stdout not piped");
    let mut reader = BufReader::with_capacity(64 * 1024, bun_stdout);
    let config = read_config(&mut reader)?;

    // Reader thread — forwards Bun stdout events. Dies when stdout closes
    // (on Bun exit or kill during reload). A new reader is spawned by ReloadHandle.
    //
    // `read_until(b'\n', &mut buf)` recycles a single `Vec<u8>` across
    // iterations rather than allocating a fresh `String` per line the way
    // `BufRead::lines()` does — `read_line` would also alloc internally on
    // each call. -1 alloc per Bun stdout frame on bursty streams.
    let reader_event_tx = event_tx.clone();
    std::thread::spawn(move || {
        let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break,
                Ok(_) => {
                    // Strip trailing \r\n / \n then UTF-8-validate. Bun
                    // emits ASCII JSON so the validation is essentially free.
                    let line_bytes = strip_trailing_eol(&buf);
                    let Ok(l) = std::str::from_utf8(line_bytes) else {
                        continue;
                    };
                    let trimmed = l.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if !trimmed.starts_with('{') {
                        eprintln!("[bun] {trimmed}");
                        continue;
                    }
                    match parse_bun_msg(trimmed) {
                        Ok(BunMsg::Return { id, ok, v, error }) => {
                            // On success, `v` carries the devalue payload. On
                            // error, `error` is a plain string we surface as
                            // the wire value (the webview's resolve hook
                            // displays it without further parsing).
                            let value = if ok {
                                v.unwrap_or_else(|| "null".into())
                            } else {
                                error.unwrap_or_else(|| "unknown error".into())
                            };
                            let _ = reader_event_tx.send(BackendEvent::Return { id, ok, value });
                        },
                        Ok(BunMsg::Yield { id, v }) => {
                            let _ = reader_event_tx.send(BackendEvent::Yield { id, value: v });
                        },
                        Ok(BunMsg::Event { name, p, to }) => {
                            let _ = reader_event_tx.send(BackendEvent::Emit {
                                name,
                                payload: p,
                                to,
                            });
                        },
                        Ok(BunMsg::EvalInFrontend { script, to }) => {
                            let _ =
                                reader_event_tx.send(BackendEvent::EvalInFrontend { script, to });
                        },
                        Ok(BunMsg::Config { .. }) => {},
                        Err(e) => tynd_log!("Parse error ({e}): {trimmed}"),
                    }
                },
                // Any error means Bun closed or crashed — the reader thread exits.
                // ReloadHandle will spawn a fresh reader if/when a new Bun starts.
                Err(_) => break,
            }
        }
        // Reader thread is exiting. If it's not because of a controlled
        // reload, this is an unexpected Bun death — the runtime crashed,
        // ran out of memory, or was OOM-killed by the OS. Surface it loudly
        // so the dev sees it instead of hitting a silent "no IPC response"
        // deadlock, and forward an error overlay so the dev-mode UI shows
        // a banner the user can read.
        if reload_in_progress.load(Ordering::SeqCst) {
            tynd_host::tynd_debug!("Bun stdout closed during reload (expected)");
        } else {
            tynd_host::tynd_error!(
                "Bun subprocess exited unexpectedly — IPC channel is dead. The runtime crashed, \
                 was OOM-killed, or terminated on its own. Restart the host to recover."
            );
            let _ = reader_event_tx.send(BackendEvent::Error {
                message: "Backend runtime exited unexpectedly. Check host logs and restart."
                    .to_string(),
            });
        }
    });

    Ok((child, bun_stdin, config))
}

fn read_config(reader: &mut BufReader<std::process::ChildStdout>) -> Result<BackendConfig, String> {
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => {
                return Err("Runtime exited before sending config (is app.start() called?)".into());
            },
            Ok(_) => {},
        }

        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            eprintln!("[bun] {trimmed}");
            continue;
        }

        match parse_bun_msg(trimmed) {
            Ok(BunMsg::Config {
                window,
                dev_url,
                frontend_dir,
                menu,
                tray,
                quit_on_last_window_closed,
            }) => {
                let icon_path = std::env::var("TYND_ICON_PATH").ok();
                return Ok(BackendConfig {
                    window: window.unwrap_or_default(),
                    dev_url,
                    frontend_dir,
                    icon_path,
                    menu,
                    tray,
                    quit_on_last_window_closed,
                });
            },
            Ok(other) => return Err(format!("Expected tynd:config, got: {other:?}")),
            Err(e) => return Err(format!("Failed to parse tynd:config: {e}\nLine: {trimmed}")),
        }
    }
}
