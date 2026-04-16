use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use serde::Deserialize;
use serde_json::Value;
use vorn_host::{
    runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent, MenuItemDef, TrayConfig},
    vorn_log,
    window::WindowConfig,
};

// ── Protocol ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum BunMsg {
    #[serde(rename = "vorn:config")]
    Config {
        window:       Option<WindowConfig>,
        #[serde(rename = "devUrl")]
        dev_url:      Option<String>,
        #[serde(rename = "frontendDir")]
        frontend_dir: Option<String>,
        #[serde(default)]
        menu:         Vec<MenuItemDef>,
        tray:         Option<TrayConfig>,
    },
    #[serde(rename = "return")]
    Return { id: String, ok: bool, value: Option<Value>, error: Option<String> },
    #[serde(rename = "event")]
    Event  { name: String, payload: Value },
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawn a Bun subprocess, read the initial config, and return a `BackendBridge`.
/// Blocks until Bun sends `vorn:config`.
///
/// When running as a compiled standalone binary (vorn build), the launcher sets
/// `VORN_BUN_PATH` to its own executable path so vorn-full uses the embedded
/// Bun runtime instead of searching the system PATH.
pub fn start(entry_path: &str) -> BackendBridge {
    // Prefer VORN_BUN_PATH (set by the compiled launcher) over system bun
    let bun_bin = std::env::var("VORN_BUN_PATH").unwrap_or_else(|_| "bun".into());

    let mut cmd = Command::new(&bun_bin);
    cmd.arg("run")
        .arg(entry_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if cfg!(debug_assertions) { Stdio::inherit() } else { Stdio::null() });

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn()
        .unwrap_or_else(|e| {
            vorn_log!("Failed to start runtime: {e}");
            if std::env::var("VORN_BUN_PATH").is_err() {
                vorn_log!("Ensure runtime is installed and in PATH");
            }
            std::process::exit(1);
        });

    let bun_stdin  = child.stdin.take().expect("Bun stdin not piped");
    let bun_stdout = child.stdout.take().expect("Bun stdout not piped");

    // Block-read the first line → vorn:config
    let mut reader = BufReader::new(bun_stdout);
    let config = read_config(&mut reader);

    // call_tx is sent from the WebView IPC handler thread — unbounded so the WebView thread never blocks.
    // event_tx is backend → main thread; unbounded so the stdout reader thread never blocks.
    let (call_tx, call_rx)   = mpsc::channel::<BackendCall>();
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>();

    // Thread: forward BackendCalls → Bun stdin
    {
        let mut stdin = bun_stdin;
        std::thread::spawn(move || {
            while let Ok(call) = call_rx.recv() {
                match call {
                    // Hot path: raw JSON from frontend IPC — forward directly,
                    // no re-serialization needed (Bun expects the same format).
                    BackendCall::Raw(json) => {
                        let _ = stdin.write_all(json.as_bytes());
                        let _ = stdin.write_all(b"\n");
                    }
                    // Lifecycle signals and any typed calls
                    BackendCall::Typed { id, fn_name, args } => {
                        let msg = match fn_name.as_str() {
                            "__vorn_on_ready__" => serde_json::json!({ "type": "vorn:ready" }),
                            "__vorn_on_close__" => serde_json::json!({ "type": "vorn:close" }),
                            _ => serde_json::json!({
                                "type": "call",
                                "id":   id,
                                "fn":   fn_name,
                                "args": args,
                            }),
                        };
                        match serde_json::to_string(&msg) {
                            Ok(mut line) => {
                                line.push('\n');
                                let _ = stdin.write_all(line.as_bytes());
                            }
                            Err(e) => eprintln!("[vorn] failed to serialize BackendCall: {e}"),
                        }
                    }
                }
            }
        });
    }

    // Thread: relay Bun stdout → BackendEvents
    std::thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    let trimmed = l.trim();
                    if trimmed.is_empty() { continue; }
                    if !trimmed.starts_with('{') {
                        eprintln!("[bun] {trimmed}");
                        continue;
                    }
                    match serde_json::from_str::<BunMsg>(trimmed) {
                        Ok(BunMsg::Return { id, ok, value, error }) => {
                            let v = if ok {
                                value.unwrap_or(Value::Null)
                            } else {
                                Value::String(error.unwrap_or_else(|| "unknown error".into()))
                            };
                            let _ = event_tx.send(BackendEvent::Return { id, ok, value: v });
                        }
                        Ok(BunMsg::Event { name, payload }) => {
                            let _ = event_tx.send(BackendEvent::Emit { name, payload });
                        }
                        Ok(BunMsg::Config { .. }) => {} // already consumed
                        Err(e) => vorn_log!("Parse error ({e}): {trimmed}"),
                    }
                }
                Err(e) => {
                    vorn_host::cleanup::run();
                    // Broken pipe / unexpected EOF means Bun shut down cleanly
                    // (user closed window). Only use exit code 1 for real IO errors.
                    use std::io::ErrorKind;
                    match e.kind() {
                        ErrorKind::BrokenPipe
                        | ErrorKind::ConnectionReset
                        | ErrorKind::UnexpectedEof => std::process::exit(0),
                        _ => std::process::exit(1),
                    }
                }
            }
        }
        vorn_host::cleanup::run();
        std::process::exit(0);
    });

    BackendBridge { config, call_tx, event_rx }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_config(reader: &mut BufReader<std::process::ChildStdout>) -> BackendConfig {
    // Skip non-JSON lines (e.g. Bun deprecation warnings leaked to stdout)
    // until we get the vorn:config message.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => {
                vorn_log!("Runtime exited before sending config");
                vorn_log!("Make sure your backend calls app.start()");
                std::process::exit(1);
            }
            Ok(_) => {}
        }

        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            eprintln!("[bun] {trimmed}");
            continue;
        }

        match serde_json::from_str::<BunMsg>(trimmed) {
            Ok(BunMsg::Config { window, dev_url, frontend_dir, menu, tray }) => {
                // VORN_ICON_PATH is set by the compiled launcher when it extracts
                // the icon from the embedded assets to a temp directory.
                let icon_path = std::env::var("VORN_ICON_PATH").ok();
                return BackendConfig {
                    window: window.unwrap_or_default(),
                    dev_url,
                    frontend_dir,
                    icon_path,
                    menu,
                    tray,
                };
            }
            Ok(other) => {
                vorn_log!("Expected vorn:config, got: {other:?}");
                std::process::exit(1);
            }
            Err(e) => {
                vorn_log!("Failed to parse vorn:config: {e}\nLine: {trimmed}");
                std::process::exit(1);
            }
        }
    }
}

