use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
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
        "copyDir" => copy_dir(args),
        "trash" => trash_path(args),
        "symlink" => symlink(args),
        "readlink" => readlink(args),
        "hardlink" => hardlink(args),
        "open" => open_handle(args),
        "seek" => seek_handle(args),
        "read" => read_handle(args),
        "write" => write_handle(args),
        "close" => close_handle(args),
        "watch" => watch(args),
        "unwatch" => unwatch(args),
        // readBinary / writeBinary intentionally route through the
        // `tynd-bin://` custom protocol (see host-rs/src/scheme_bin.rs),
        // not JSON IPC — they're zero-copy on the wire.
        _ => Err(format!("fs.{method}: unknown method")),
    }
}

// --- File handles -------------------------------------------------------
// Stateful seek/read/write — each `open` returns a numeric id that
// subsequent ops reference. Closed explicitly by `close`. Base64 on the
// wire for binary payloads (matches other small-blob APIs); for multi-MB
// payloads prefer `readBinary` / `writeBinary` via the bin scheme.

// Each file sits behind its own mutex so blocking I/O on one handle
// doesn't stall concurrent opens/closes or operations on other handles.
// The outer map lock is only held during insert / remove / lookup.
static HANDLES: OnceLock<Mutex<ahash::HashMap<u64, std::sync::Arc<Mutex<File>>>>> = OnceLock::new();
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

fn handles() -> &'static Mutex<ahash::HashMap<u64, std::sync::Arc<Mutex<File>>>> {
    HANDLES.get_or_init(|| Mutex::new(ahash::HashMap::default()))
}

fn open_handle(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let read = args.get("read").and_then(Value::as_bool).unwrap_or(true);
    let write = args.get("write").and_then(Value::as_bool).unwrap_or(false);
    let append = args.get("append").and_then(Value::as_bool).unwrap_or(false);
    let create = args.get("create").and_then(Value::as_bool).unwrap_or(false);
    let truncate = args
        .get("truncate")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let file = OpenOptions::new()
        .read(read)
        .write(write)
        .append(append)
        .create(create)
        .truncate(truncate)
        .open(path)
        .map_err(|e| format!("fs.open({path}): {e}"))?;

    let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::SeqCst);
    handles()
        .lock()
        .insert(id, std::sync::Arc::new(Mutex::new(file)));
    Ok(json!({ "id": id }))
}

/// Look up the handle, clone the Arc, drop the outer lock, then perform
/// the I/O under the per-handle mutex. Blocking syscalls never hold the
/// map lock.
fn with_handle<R>(args: &Value, f: impl FnOnce(&mut File) -> R) -> Result<R, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs: missing 'id'".to_string())?;
    let file = handles()
        .lock()
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("fs: handle {id} not open"))?;
    let mut guard = file.lock();
    Ok(f(&mut guard))
}

fn seek_handle(args: &Value) -> Result<Value, String> {
    let offset = args
        .get("offset")
        .and_then(Value::as_i64)
        .ok_or_else(|| "fs.seek: missing 'offset'".to_string())?;
    let from = match args.get("from").and_then(Value::as_str).unwrap_or("start") {
        "current" => SeekFrom::Current(offset),
        "end" => SeekFrom::End(offset),
        _ => SeekFrom::Start(offset.max(0) as u64),
    };
    let new = with_handle(args, |f| f.seek(from))?.map_err(|e| format!("fs.seek: {e}"))?;
    Ok(json!({ "position": new }))
}

fn read_handle(args: &Value) -> Result<Value, String> {
    let len = args
        .get("length")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs.read: missing 'length'".to_string())? as usize;
    let mut buf = vec![0u8; len];
    let n = with_handle(args, |f| f.read(&mut buf))?.map_err(|e| format!("fs.read: {e}"))?;
    buf.truncate(n);
    Ok(json!({ "bytes": STANDARD.encode(&buf), "read": n }))
}

fn write_handle(args: &Value) -> Result<Value, String> {
    let b64 = args
        .get("bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| "fs.write: missing 'bytes'".to_string())?;
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| format!("fs.write: bad base64 — {e}"))?;
    let n = with_handle(args, |f| f.write(&bytes))?.map_err(|e| format!("fs.write: {e}"))?;
    Ok(json!({ "written": n }))
}

fn close_handle(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs.close: missing 'id'".to_string())?;
    Ok(Value::Bool(handles().lock().remove(&id).is_some()))
}

fn copy_dir(args: &Value) -> Result<Value, String> {
    let from = path_arg(args, "from")?;
    let to = path_arg(args, "to")?;
    copy_dir_recursive(Path::new(from), Path::new(to))
        .map_err(|e| format!("fs.copyDir({from} -> {to}): {e}"))?;
    Ok(Value::Null)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_child = entry.path();
        let dst_child = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_child, &dst_child)?;
        } else if ty.is_file() {
            fs::copy(&src_child, &dst_child)?;
        }
        // symlinks skipped — explicit `symlink` API handles those.
    }
    Ok(())
}

fn symlink(args: &Value) -> Result<Value, String> {
    let target = path_arg(args, "target")?;
    let link = path_arg(args, "link")?;
    platform_symlink(target, link).map_err(|e| format!("fs.symlink: {e}"))?;
    Ok(Value::Null)
}

#[cfg(unix)]
fn platform_symlink(target: &str, link: &str) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn platform_symlink(target: &str, link: &str) -> std::io::Result<()> {
    // Windows distinguishes file vs dir symlinks — choose based on what the
    // target currently is. If the target doesn't exist, fall back to file
    // (most common) so a later `mkdir` doesn't break the link.
    let p = Path::new(target);
    if p.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

fn readlink(args: &Value) -> Result<Value, String> {
    let path = path_arg(args, "path")?;
    let target = fs::read_link(path).map_err(|e| format!("fs.readlink({path}): {e}"))?;
    Ok(Value::String(target.to_string_lossy().into_owned()))
}

fn hardlink(args: &Value) -> Result<Value, String> {
    let original = path_arg(args, "original")?;
    let link = path_arg(args, "link")?;
    fs::hard_link(original, link).map_err(|e| format!("fs.hardlink: {e}"))?;
    Ok(Value::Null)
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
    let path = args
        .get(name)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("fs: missing '{name}' argument"))?;
    super::security::check_fs(path)?;
    Ok(path)
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

    #[test]
    fn file_handle_round_trip() {
        let p = tmp_path("handle.bin");
        let open = open_handle(&json!({
            "path": &p,
            "read": true,
            "write": true,
            "create": true,
            "truncate": true,
        }))
        .unwrap();
        let id = open["id"].as_u64().unwrap();

        let write = write_handle(&json!({ "id": id, "bytes": STANDARD.encode(b"hello") })).unwrap();
        assert_eq!(write["written"].as_u64().unwrap(), 5);

        seek_handle(&json!({ "id": id, "offset": 0, "from": "start" })).unwrap();
        let read = read_handle(&json!({ "id": id, "length": 5 })).unwrap();
        assert_eq!(read["read"].as_u64().unwrap(), 5);
        let bytes = STANDARD.decode(read["bytes"].as_str().unwrap()).unwrap();
        assert_eq!(bytes, b"hello");

        assert_eq!(
            close_handle(&json!({ "id": id })).unwrap(),
            Value::Bool(true)
        );
        fs::remove_file(&p).unwrap();
    }

    #[test]
    fn copy_dir_copies_nested_files() {
        let src = tmp_path("copysrc");
        let dst = tmp_path("copydst");
        let nested = format!("{src}/a/b");
        mkdir(&json!({ "path": &nested, "recursive": true })).unwrap();
        write_text(&json!({ "path": format!("{nested}/f.txt"), "content": "x" })).unwrap();
        copy_dir(&json!({ "from": &src, "to": &dst })).unwrap();
        assert_eq!(fs::read_to_string(format!("{dst}/a/b/f.txt")).unwrap(), "x");
        remove(&json!({ "path": &src, "recursive": true })).unwrap();
        remove(&json!({ "path": &dst, "recursive": true })).unwrap();
    }

    #[test]
    fn hardlink_creates_second_path_same_content() {
        let a = tmp_path("orig.txt");
        let b = tmp_path("link.txt");
        write_text(&json!({ "path": &a, "content": "same" })).unwrap();
        hardlink(&json!({ "original": &a, "link": &b })).unwrap();
        assert_eq!(fs::read_to_string(&b).unwrap(), "same");
        fs::remove_file(&a).unwrap();
        fs::remove_file(&b).unwrap();
    }
}
