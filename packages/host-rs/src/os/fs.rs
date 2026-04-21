use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use super::events;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "readText" => read_text(args),
        "writeText" => write_text(args),
        "exists" => exists(args),
        "stat" => stat(args),
        "readDir" => read_dir(args),
        "mkdir" => mkdir(args),
        "remove" => remove(args),
        "rename" => rename(args),
        "copy" => copy(args),
        "trash" => trash_path(args),
        "watch" => watch(args),
        "unwatch" => unwatch(args),
        // readBinary / writeBinary intentionally route through the
        // `tynd-bin://` custom protocol (see host-rs/src/scheme_bin.rs),
        // not JSON IPC — they're zero-copy on the wire.
        _ => Err(format!("fs.{method}: unknown method")),
    }
}

// --- FS watcher ---------------------------------------------------------
// Each `watch` call owns a `RecommendedWatcher` (ReadDirectoryChangesW /
// FSEvents / inotify) stored behind a monotonically-increasing numeric id.
// `unwatch(id)` drops the watcher, which tears down the OS handle.

static WATCHERS: OnceLock<Mutex<ahash::HashMap<u64, RecommendedWatcher>>> = OnceLock::new();
static NEXT_WATCH_ID: AtomicU64 = AtomicU64::new(1);

fn watchers() -> &'static Mutex<ahash::HashMap<u64, RecommendedWatcher>> {
    WATCHERS.get_or_init(|| Mutex::new(ahash::HashMap::default()))
}

fn kind_label(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Create(_) => "create",
        EventKind::Modify(notify::event::ModifyKind::Name(_)) => "rename",
        EventKind::Modify(_) => "modify",
        EventKind::Remove(_) => "delete",
        _ => "other",
    }
}

fn watch(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let id = NEXT_WATCH_ID.fetch_add(1, Ordering::SeqCst);

    let mut watcher =
        notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
            Ok(event) => {
                let kind = kind_label(event.kind);
                for p in &event.paths {
                    events::emit(
                        "fs:change",
                        &json!({
                            "id": id,
                            "kind": kind,
                            "path": p.to_string_lossy(),
                        }),
                    );
                }
            },
            Err(e) => {
                events::emit(
                    "fs:change",
                    &json!({ "id": id, "kind": "error", "error": e.to_string() }),
                );
            },
        })
        .map_err(|e| format!("fs.watch: {e}"))?;

    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher
        .watch(Path::new(path), mode)
        .map_err(|e| format!("fs.watch({path}): {e}"))?;

    watchers().lock().insert(id, watcher);
    Ok(json!({ "id": id }))
}

fn unwatch(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs.unwatch: missing 'id'".to_string())?;
    let removed = watchers().lock().remove(&id);
    Ok(Value::Bool(removed.is_some()))
}

fn path_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("fs: missing '{name}' argument"))
}

fn read_text(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let s = fs::read_to_string(path).map_err(|e| format!("fs.readText({path}): {e}"))?;
    Ok(Value::String(s))
}

fn write_text(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "fs.writeText: missing 'content'".to_string())?;
    if args
        .get("createDirs")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        if let Some(parent) = Path::new(path).parent() {
            fs::create_dir_all(parent).map_err(|e| format!("fs.writeText: {e}"))?;
        }
    }
    fs::write(path, content).map_err(|e| format!("fs.writeText({path}): {e}"))?;
    Ok(Value::Null)
}

fn exists(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    Ok(Value::Bool(Path::new(path).exists()))
}

fn stat(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let meta = fs::metadata(path).map_err(|e| format!("fs.stat({path}): {e}"))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    Ok(json!({
        "size": meta.len(),
        "isFile": meta.is_file(),
        "isDir": meta.is_dir(),
        "isSymlink": meta.file_type().is_symlink(),
        "mtime": mtime,
    }))
}

fn read_dir(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let entries = fs::read_dir(path).map_err(|e| format!("fs.readDir({path}): {e}"))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("fs.readDir: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let ft = entry.file_type().map_err(|e| format!("fs.readDir: {e}"))?;
        out.push(json!({
            "name": name,
            "isFile": ft.is_file(),
            "isDir": ft.is_dir(),
            "isSymlink": ft.is_symlink(),
        }));
    }
    Ok(Value::Array(out))
}

fn mkdir(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if recursive {
        fs::create_dir_all(path).map_err(|e| format!("fs.mkdir({path}): {e}"))?;
    } else {
        fs::create_dir(path).map_err(|e| format!("fs.mkdir({path}): {e}"))?;
    }
    Ok(Value::Null)
}

fn remove(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let p = Path::new(path);
    let meta = fs::symlink_metadata(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
    if meta.is_dir() {
        if recursive {
            fs::remove_dir_all(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
        } else {
            fs::remove_dir(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
        }
    } else {
        fs::remove_file(p).map_err(|e| format!("fs.remove({path}): {e}"))?;
    }
    Ok(Value::Null)
}

fn rename(args: &Value) -> Result<Value, String> {
    let from = path_arg(args, "from")?;
    let to = path_arg(args, "to")?;
    fs::rename(from, to).map_err(|e| format!("fs.rename({from} -> {to}): {e}"))?;
    Ok(Value::Null)
}

fn copy(args: &Value) -> Result<Value, String> {
    let from = path_arg(args, "from")?;
    let to = path_arg(args, "to")?;
    fs::copy(from, to).map_err(|e| format!("fs.copy({from} -> {to}): {e}"))?;
    Ok(Value::Null)
}

fn trash_path(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    trash::delete(path).map_err(|e| format!("fs.trash({path}): {e}"))?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_path(name: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        env::temp_dir()
            .join(format!("tynd-fs-test-{ts}-{name}"))
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn text_roundtrip() {
        let p = tmp_path("text.txt");
        write_text(&json!({ "path": &p, "content": "héllo" })).unwrap();
        let v = read_text(&json!({ "path": &p })).unwrap();
        assert_eq!(v.as_str().unwrap(), "héllo");
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn stat_reports_size_and_isfile() {
        let p = tmp_path("stat.txt");
        write_text(&json!({ "path": &p, "content": "abcd" })).unwrap();
        let s = stat(&json!({ "path": &p })).unwrap();
        assert_eq!(s["size"].as_u64().unwrap(), 4);
        assert!(s["isFile"].as_bool().unwrap());
        assert!(!s["isDir"].as_bool().unwrap());
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn mkdir_recursive_and_remove() {
        let base = tmp_path("dirs");
        let nested = format!("{base}/a/b/c");
        mkdir(&json!({ "path": &nested, "recursive": true })).unwrap();
        assert!(Path::new(&nested).exists());
        remove(&json!({ "path": &base, "recursive": true })).unwrap();
        assert!(!Path::new(&base).exists());
    }

    #[test]
    fn missing_path_arg_errors() {
        assert!(read_text(&json!({})).is_err());
    }
}
