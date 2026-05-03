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

use crate::bun_ipc::WireFormat;

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
    /// Wire format negotiated for this Bun lifetime — propagated to the
    /// freshly-spawned child on reload so the format stays stable across
    /// the swap (otherwise an env-var change between launches could
    /// corrupt the in-flight call queue).
    wire_fmt: WireFormat,
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
            self.wire_fmt,
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

    // Pick the wire format once at process start. Both directions
    // (host→Bun stdin, Bun→host stdout) use the same format — the
    // Bun child inherits `TYND_IPC_FORMAT` set by `spawn_bun`.
    let wire_fmt = WireFormat::from_env();
    tynd_host::tynd_debug!("Bun IPC wire format: {}", wire_fmt.as_env_value());

    let (child, stdin, config) = match spawn_bun(
        entry_path,
        &event_tx,
        reload_in_progress.clone(),
        wire_fmt,
    ) {
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
                push_call_line(&mut burst, call, wire_fmt);
                while let Ok(extra) = call_rx.try_recv() {
                    push_call_line(&mut burst, extra, wire_fmt);
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
        wire_fmt,
    };
    (bridge, reload)
}

/// Append the wire-encoded frame for one BackendCall to the coalescing
/// buffer. Used by the stdin forwarder (B18) to batch multiple frames
/// into a single `write_all` syscall.
///
/// The wire format is picked once at process start (see
/// [`bun_ipc::WireFormat::from_env`]). On the JSON path each frame is
/// `\n`-terminated; on the CBOR path each frame is length-prefixed
/// (4-byte u32 LE header + body). Both paths funnel through the same
/// match arm so the coalescing semantics stay identical.
fn push_call_line(out: &mut Vec<u8>, call: BackendCall, fmt: WireFormat) {
    // `Raw` is a pre-encoded JSON line forwarded straight from the
    // webview shim. It only makes sense on the JSON wire — on the
    // binary wire the webview shim will emit pre-encoded CBOR (one
    // frame per IPC message) or call through the typed path.
    match call {
        BackendCall::Raw(s) => match fmt {
            WireFormat::Json => {
                let mut bytes = s.into_bytes();
                bytes.push(b'\n');
                out.extend_from_slice(&bytes);
            },
            #[cfg(feature = "binary-ipc")]
            WireFormat::Cbor => {
                // Webview pre-encoded JSON can't be forwarded verbatim
                // on the binary wire. Decode → re-encode as CBOR. This
                // is only the cold edge case where a webview opted into
                // binary IPC but a plugin still emits a JSON Raw frame;
                // typed callers go through the no-decode path below.
                match serde_json::from_str::<Value>(&s) {
                    Ok(v) => {
                        let mut body: Vec<u8> = Vec::with_capacity(s.len());
                        if let Err(e) = ciborium::ser::into_writer(&v, &mut body) {
                            tynd_host::tynd_error!("Raw → CBOR re-encode failed: {e}");
                            return;
                        }
                        crate::bun_ipc::push_cbor_frame_into(out, &body);
                    },
                    Err(e) => {
                        tynd_host::tynd_error!("Raw frame is not valid JSON: {e}");
                    },
                }
            },
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
            encode_wire_msg(out, &msg, fmt);
        },
        BackendCall::Cancel { id } => {
            encode_wire_msg(out, &WireMsg::Cancel { id: &id }, fmt);
        },
        BackendCall::Ack { id, n } => {
            encode_wire_msg(out, &WireMsg::Ack { id: &id, n }, fmt);
        },
    }
}

/// Format-aware encoder for a single `WireMsg` — JSON line on the
/// legacy path, length-prefixed CBOR frame on the binary path. Errors
/// are logged and the frame is dropped (matches the previous JSON
/// path's `Err(_) => return` semantics — a serialize failure on
/// IPC is unrecoverable, the dispatch is already lost).
fn encode_wire_msg(out: &mut Vec<u8>, msg: &WireMsg<'_>, fmt: WireFormat) {
    match fmt {
        WireFormat::Json => match serde_json::to_string(msg) {
            Ok(mut s) => {
                s.push('\n');
                out.extend_from_slice(s.as_bytes());
            },
            Err(e) => {
                tynd_host::tynd_error!("failed to serialize WireMsg as JSON: {e}");
            },
        },
        #[cfg(feature = "binary-ipc")]
        WireFormat::Cbor => {
            let mut body: Vec<u8> = Vec::with_capacity(128);
            if let Err(e) = ciborium::ser::into_writer(msg, &mut body) {
                tynd_host::tynd_error!("failed to serialize WireMsg as CBOR: {e}");
                return;
            }
            crate::bun_ipc::push_cbor_frame_into(out, &body);
        },
    }
}

/// Spawn a Bun subprocess, wait for `tynd:config`, spawn the stdout reader thread,
/// and return the child handle + stdin + parsed config.
fn spawn_bun(
    entry_path: &str,
    event_tx: &mpsc::Sender<BackendEvent>,
    reload_in_progress: Arc<AtomicBool>,
    wire_fmt: WireFormat,
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

    // Make the wire-format contract explicit on the child env so the
    // Bun runtime's stdin reader picks the matching decoder. Set even
    // for `Json` so a stale `cbor` value left in the parent shell
    // can't accidentally cross over to the child.
    cmd.env("TYND_IPC_FORMAT", wire_fmt.as_env_value());

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
    let config = read_config(&mut reader, wire_fmt)?;

    // Reader thread — forwards Bun stdout events. Dies when stdout closes
    // (on Bun exit or kill during reload). A new reader is spawned by ReloadHandle.
    //
    // `read_until(b'\n', &mut buf)` recycles a single `Vec<u8>` across
    // iterations rather than allocating a fresh `String` per line the way
    // `BufRead::lines()` does — `read_line` would also alloc internally on
    // each call. -1 alloc per Bun stdout frame on bursty streams.
    let reader_event_tx = event_tx.clone();
    std::thread::spawn(move || {
        match wire_fmt {
            WireFormat::Json => json_reader_loop(&mut reader, &reader_event_tx),
            #[cfg(feature = "binary-ipc")]
            WireFormat::Cbor => cbor_reader_loop(&mut reader, &reader_event_tx),
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

/// Forward a parsed `BunMsg` to the host event channel. Single shared
/// helper between the JSON and CBOR reader loops so both paths produce
/// identical `BackendEvent` shapes.
fn forward_bun_msg(msg: BunMsg, tx: &mpsc::Sender<BackendEvent>) {
    match msg {
        BunMsg::Return { id, ok, v, error } => {
            // On success, `v` carries the devalue payload. On error,
            // `error` is a plain string we surface as the wire value
            // (the webview's resolve hook displays it without further
            // parsing).
            let value = if ok {
                v.unwrap_or_else(|| "null".into())
            } else {
                error.unwrap_or_else(|| "unknown error".into())
            };
            let _ = tx.send(BackendEvent::Return { id, ok, value });
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
        BunMsg::Config { .. } => {
            // Stale config frame mid-stream — ignore (would only happen
            // if Bun re-emitted config after handshake, which is a
            // contract violation but not worth crashing over).
        },
    }
}

/// Legacy `\n`-delimited JSON reader. Recycles a single `Vec<u8>`
/// across iterations rather than allocating a fresh `String` per line
/// the way `BufRead::lines()` does — `read_line` would also alloc
/// internally on each call. -1 alloc per Bun stdout frame on bursty
/// streams.
fn json_reader_loop(
    reader: &mut BufReader<std::process::ChildStdout>,
    tx: &mpsc::Sender<BackendEvent>,
) {
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
                    Ok(msg) => forward_bun_msg(msg, tx),
                    Err(e) => tynd_log!("Parse error ({e}): {trimmed}"),
                }
            },
            // Any error means Bun closed or crashed — the reader thread exits.
            // ReloadHandle will spawn a fresh reader if/when a new Bun starts.
            Err(_) => break,
        }
    }
}

/// Length-prefixed CBOR reader. Each frame is a 4-byte u32 LE header
/// followed by a CBOR-encoded `BunMsg` body. Framing errors break
/// the loop so the surrounding thread surfaces the dead-IPC banner;
/// per-frame decode errors are logged and skipped (matches the JSON
/// path's "log and keep going" semantic for malformed frames).
#[cfg(feature = "binary-ipc")]
fn cbor_reader_loop(
    reader: &mut BufReader<std::process::ChildStdout>,
    tx: &mpsc::Sender<BackendEvent>,
) {
    loop {
        let body = match crate::bun_ipc::read_cbor_frame(reader) {
            Ok(Some(b)) => b,
            Ok(None) => break, // clean EOF — Bun closed stdout
            Err(e) => {
                tynd_host::tynd_error!("CBOR framing error on Bun stdout: {e}");
                break;
            },
        };
        match ciborium::de::from_reader::<BunMsg, _>(&body[..]) {
            Ok(msg) => forward_bun_msg(msg, tx),
            Err(e) => tynd_log!("CBOR decode error ({e}): {} bytes", body.len()),
        }
    }
}

fn read_config(
    reader: &mut BufReader<std::process::ChildStdout>,
    fmt: WireFormat,
) -> Result<BackendConfig, String> {
    let msg = read_first_bun_msg(reader, fmt)?;
    match msg {
        BunMsg::Config {
            window,
            dev_url,
            frontend_dir,
            menu,
            tray,
            quit_on_last_window_closed,
        } => {
            let icon_path = std::env::var("TYND_ICON_PATH").ok();
            Ok(BackendConfig {
                window: window.unwrap_or_default(),
                dev_url,
                frontend_dir,
                icon_path,
                menu,
                tray,
                quit_on_last_window_closed,
            })
        },
        other => Err(format!("Expected tynd:config, got: {other:?}")),
    }
}

/// Read frames from Bun stdout until we land on a `BunMsg`. On the
/// JSON wire, plain log lines (anything not starting with `{`) are
/// re-emitted to stderr so devs see them; on the binary wire there's
/// no such fallback (binary log lines would corrupt the framer), so
/// any decode error returns. Skips empty lines on the JSON path.
fn read_first_bun_msg(
    reader: &mut BufReader<std::process::ChildStdout>,
    fmt: WireFormat,
) -> Result<BunMsg, String> {
    match fmt {
        WireFormat::Json => loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => {
                    return Err(
                        "Runtime exited before sending config (is app.start() called?)".into(),
                    );
                },
                Ok(_) => {},
            }
            let trimmed = line.trim();
            if !trimmed.starts_with('{') {
                eprintln!("[bun] {trimmed}");
                continue;
            }
            return parse_bun_msg(trimmed)
                .map_err(|e| format!("Failed to parse tynd:config: {e}\nLine: {trimmed}"));
        },
        #[cfg(feature = "binary-ipc")]
        WireFormat::Cbor => {
            let body = crate::bun_ipc::read_cbor_frame(reader)
                .map_err(|e| format!("CBOR framing error reading config: {e}"))?
                .ok_or_else(|| {
                    "Runtime exited before sending config (is app.start() called?)".to_string()
                })?;
            ciborium::de::from_reader::<BunMsg, _>(&body[..])
                .map_err(|e| format!("Failed to decode tynd:config CBOR frame: {e}"))
        },
    }
}
