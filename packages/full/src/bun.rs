use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

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

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum BunMsg {
    #[serde(rename = "vorn:config")]
    Config {
        window: Option<WindowConfig>,
        #[serde(rename = "devUrl")]
        dev_url: Option<String>,
        #[serde(rename = "frontendDir")]
        frontend_dir: Option<String>,
        #[serde(default)]
        menu: Vec<MenuItemDef>,
        tray: Option<TrayConfig>,
    },
    #[serde(rename = "return")]
    Return {
        id: String,
        ok: bool,
        value: Option<Value>,
        error: Option<String>,
    },
    #[serde(rename = "event")]
    Event { name: String, payload: Value },
}

/// Handle for dev-mode hot-reload of the Bun subprocess without tearing down
/// the host process or the WebView window.
#[derive(Clone)]
pub(crate) struct ReloadHandle {
    entry_path: String,
    stdin_slot: Arc<Mutex<Option<ChildStdin>>>,
    child_slot: Arc<Mutex<Option<Child>>>,
    event_tx: mpsc::Sender<BackendEvent>,
}

impl ReloadHandle {
    /// Kill the current Bun subprocess and spawn a new one. The WebView stays
    /// alive — after the new backend is ready, `BackendEvent::Reload` is sent
    /// so the host soft-reloads the page.
    pub(crate) fn reload(&self) {
        // 1. Kill the old child — this closes its stdout, unblocking the reader thread.
        if let Some(mut child) = self.child_slot.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // 2. Drop old stdin — forwarder will hold the mutex and see None.
        *self.stdin_slot.lock().unwrap() = None;

        // 3. Spawn a fresh Bun. Reader thread for the new stdout is spawned inside.
        match spawn_bun(&self.entry_path, self.event_tx.clone()) {
            Ok((child, stdin, _config)) => {
                *self.child_slot.lock().unwrap() = Some(child);
                *self.stdin_slot.lock().unwrap() = Some(stdin);
                // Tell the host to soft-reload the WebView — preserves window
                // position, size, and the wry webview instance itself.
                let _ = self.event_tx.send(BackendEvent::Reload);
            },
            Err(e) => {
                let _ = self.event_tx.send(BackendEvent::Error {
                    message: format!("Failed to reload backend: {e}"),
                });
            },
        }
    }
}

/// Spawn a Bun subprocess, read the initial config, and return a `BackendBridge`
/// plus a `ReloadHandle` for dev-mode hot reload.
///
/// When running as a compiled standalone binary (vorn build), the launcher sets
/// `VORN_BUN_PATH` to its own executable path so vorn-full uses the embedded
/// Bun runtime instead of searching the system PATH.
pub(crate) fn start(entry_path: &str) -> (BackendBridge, ReloadHandle) {
    let (call_tx, call_rx) = mpsc::channel::<BackendCall>();
    let (event_tx, event_rx) = mpsc::channel::<BackendEvent>();

    let (child, stdin, config) = match spawn_bun(entry_path, event_tx.clone()) {
        Ok(t) => t,
        Err(e) => {
            vorn_log!("{e}");
            std::process::exit(1);
        },
    };

    let stdin_slot: Arc<Mutex<Option<ChildStdin>>> = Arc::new(Mutex::new(Some(stdin)));
    let child_slot: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(Some(child)));

    // Forwarder: call_rx → current Bun stdin (under mutex so reload can swap it)
    {
        let stdin_slot = stdin_slot.clone();
        std::thread::spawn(move || {
            while let Ok(call) = call_rx.recv() {
                let line = match call {
                    BackendCall::Raw(mut s) => {
                        s.push('\n');
                        s.into_bytes()
                    },
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
                            Ok(mut s) => {
                                s.push('\n');
                                s.into_bytes()
                            },
                            Err(e) => {
                                eprintln!("[vorn] failed to serialize BackendCall: {e}");
                                continue;
                            },
                        }
                    },
                };
                // Drop the message if Bun is mid-reload (stdin slot is None).
                if let Some(stdin) = stdin_slot.lock().unwrap().as_mut() {
                    let _ = stdin.write_all(&line);
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
    };
    (bridge, reload)
}

/// Spawn a Bun subprocess, wait for `vorn:config`, spawn the stdout reader thread,
/// and return the child handle + stdin + parsed config.
fn spawn_bun(
    entry_path: &str,
    event_tx: mpsc::Sender<BackendEvent>,
) -> Result<(Child, ChildStdin, BackendConfig), String> {
    let bun_bin = std::env::var("VORN_BUN_PATH").unwrap_or_else(|_| "bun".into());

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
        if std::env::var("VORN_BUN_PATH").is_err() {
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
    std::thread::spawn(move || {
        for line in reader.lines() {
            match line {
                Ok(l) => {
                    let trimmed = l.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if !trimmed.starts_with('{') {
                        eprintln!("[bun] {trimmed}");
                        continue;
                    }
                    match serde_json::from_str::<BunMsg>(trimmed) {
                        Ok(BunMsg::Return {
                            id,
                            ok,
                            value,
                            error,
                        }) => {
                            let v = if ok {
                                value.unwrap_or(Value::Null)
                            } else {
                                Value::String(error.unwrap_or_else(|| "unknown error".into()))
                            };
                            let _ = event_tx.send(BackendEvent::Return { id, ok, value: v });
                        },
                        Ok(BunMsg::Event { name, payload }) => {
                            let _ = event_tx.send(BackendEvent::Emit { name, payload });
                        },
                        Ok(BunMsg::Config { .. }) => {},
                        Err(e) => vorn_log!("Parse error ({e}): {trimmed}"),
                    }
                },
                // Any error means Bun closed or crashed — the reader thread exits.
                // ReloadHandle will spawn a fresh reader if/when a new Bun starts.
                Err(_) => return,
            }
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

        match serde_json::from_str::<BunMsg>(trimmed) {
            Ok(BunMsg::Config {
                window,
                dev_url,
                frontend_dir,
                menu,
                tray,
            }) => {
                let icon_path = std::env::var("VORN_ICON_PATH").ok();
                return Ok(BackendConfig {
                    window: window.unwrap_or_default(),
                    dev_url,
                    frontend_dir,
                    icon_path,
                    menu,
                    tray,
                });
            },
            Ok(other) => return Err(format!("Expected vorn:config, got: {other:?}")),
            Err(e) => return Err(format!("Failed to parse vorn:config: {e}\nLine: {trimmed}")),
        }
    }
}
