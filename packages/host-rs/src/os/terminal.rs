//! Embedded PTY terminals. Output is streamed as `terminal:data` events
//! (base64), termination as `terminal:exit { code }`.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use dashmap::DashMap;
use parking_lot::Mutex;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use super::events;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct Session {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    /// Set by `kill()` so the child-watcher thread can suppress the
    /// synthetic `terminal:exit` event — the caller already knows.
    killed: Arc<AtomicBool>,
}

// PTY handles aren't Sync on their own, so each session is locked
// individually — lookups stay lock-free, only the owning session serialises.
type Sessions = DashMap<u64, Mutex<Session>>;

fn sessions() -> &'static Sessions {
    static S: OnceLock<Sessions> = OnceLock::new();
    S.get_or_init(DashMap::new)
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "spawn" => spawn(args),
        "write" => write(args),
        "resize" => resize(args),
        "kill" => kill(args),
        "list" => list(),
        _ => Err(format!("terminal.{method}: unknown method")),
    }
}

fn spawn(args: &Value) -> Result<Value, String> {
    let shell = args
        .get("shell")
        .and_then(Value::as_str)
        .map_or_else(default_shell, String::from);
    let argv: Vec<String> = args
        .get("args")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16;
    let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("terminal.spawn: openpty: {e}"))?;

    let mut cmd = CommandBuilder::new(&shell);
    cmd.args(&argv);
    if let Some(cwd) = args.get("cwd").and_then(Value::as_str) {
        cmd.cwd(cwd);
    }
    if let Some(env_obj) = args.get("env").and_then(Value::as_object) {
        for (k, v) in env_obj {
            if let Some(s) = v.as_str() {
                cmd.env(k, s);
            }
        }
    }

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("terminal.spawn: spawn: {e}"))?;
    drop(pair.slave);

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("terminal.spawn: clone reader: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("terminal.spawn: take writer: {e}"))?;

    let killed = Arc::new(AtomicBool::new(false));
    sessions().insert(
        id,
        Mutex::new(Session {
            master: pair.master,
            writer,
            killed: killed.clone(),
        }),
    );

    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let encoded = STANDARD.encode(&buf[..n]);
                    events::emit("terminal:data", &json!({ "id": id, "data": encoded }));
                },
            }
        }
    });

    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| {
            if s.success() {
                Some(0u32)
            } else {
                s.exit_code().into()
            }
        });
        sessions().remove(&id);
        if !killed.load(Ordering::Acquire) {
            events::emit("terminal:exit", &json!({ "id": id, "code": code }));
        }
    });

    Ok(json!({ "id": id }))
}

fn write(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "terminal.write: missing 'id'".to_string())?;
    let data_b64 = args
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "terminal.write: missing base64 'data'".to_string())?;
    let bytes = STANDARD
        .decode(data_b64)
        .map_err(|e| format!("terminal.write: invalid base64: {e}"))?;

    let session_cell = sessions()
        .get(&id)
        .ok_or_else(|| format!("terminal.write: session {id} not found"))?;
    let mut session = session_cell.lock();
    session
        .writer
        .write_all(&bytes)
        .map_err(|e| format!("terminal.write: {e}"))?;
    session.writer.flush().map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

fn resize(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "terminal.resize: missing 'id'".to_string())?;
    let cols = args.get("cols").and_then(Value::as_u64).unwrap_or(80) as u16;
    let rows = args.get("rows").and_then(Value::as_u64).unwrap_or(24) as u16;

    let session_cell = sessions()
        .get(&id)
        .ok_or_else(|| format!("terminal.resize: session {id} not found"))?;
    let session = session_cell.lock();
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("terminal.resize: {e}"))?;
    Ok(Value::Null)
}

fn kill(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "terminal.kill: missing 'id'".to_string())?;
    // Flag the watcher first so the race between this remove() and the
    // child-exit watcher doesn't emit a stray `terminal:exit` event for a
    // session the caller explicitly tore down.
    if let Some(entry) = sessions().get(&id) {
        entry.lock().killed.store(true, Ordering::Release);
    }
    sessions().remove(&id);
    Ok(Value::Null)
}

#[allow(clippy::unnecessary_wraps)] // dispatch expects Result — uniform return shape
fn list() -> Result<Value, String> {
    let ids: Vec<Value> = sessions()
        .iter()
        .map(|r| Value::Number((*r.key()).into()))
        .collect();
    Ok(Value::Array(ids))
}

fn default_shell() -> String {
    #[cfg(target_os = "windows")]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}
