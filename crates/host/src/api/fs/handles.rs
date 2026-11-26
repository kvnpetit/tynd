//! `fs.open` / `seek` / `read` / `write` / `close`: a registry of open
//! files, each owned by the webview (or backend) that opened it.

use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock,
    },
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cap_std::fs::OpenOptions;
use dashmap::DashMap;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::ipc::error::IpcError;

use super::{
    ops::{bool_arg, string_arg},
    roots::resolve,
};
use crate::ipc::owner::{self, Owner};

/// Hard cap on a single `fs.read` request. Caller-controlled `length`
/// flows directly into `READ_SCRATCH.resize(len, 0)`, so without a bound
/// a malicious frontend could ask for multi-GiB and force the host to
/// commit (and zero-fill) that much per worker thread.
const MAX_READ_LEN: usize = 16 * 1024 * 1024;

/// Hard cap on a single `fs.write` request. Caller-controlled base64
/// `bytes` decodes to ~3/4 its size; without a bound a single IPC call
/// could trigger a multi-GiB allocation in `STANDARD.decode`. Mirrors
/// `MAX_READ_LEN` for symmetry.
const MAX_WRITE_LEN: usize = 16 * 1024 * 1024;

// Each handle has its own mutex, so blocking I/O on one file never stalls
// the map.
static HANDLES: LazyLock<DashMap<u64, Handle>> = LazyLock::new(DashMap::default);
static NEXT_HANDLE_ID: AtomicU64 = AtomicU64::new(1);

/// An open file and the webview identity that opened it (`None` for the backend).
struct Handle {
    owner: Option<Owner>,
    file: Arc<Mutex<File>>,
}

impl Handle {
    /// Whether the calling webview may use this handle. A refusal is answered
    /// like an unknown id.
    fn permitted(&self) -> bool {
        owner::permits(self.owner.as_ref())
    }
}

pub(super) fn open_handle(args: &Value) -> Result<Value, String> {
    let path_str = string_arg(args, "path")?;
    let (dir, rel) = resolve(path_str)?;

    let mut opts = OpenOptions::new();
    opts.read(bool_arg(args, "read", true))
        .write(bool_arg(args, "write", false))
        .append(bool_arg(args, "append", false))
        .create(bool_arg(args, "create", false))
        .truncate(bool_arg(args, "truncate", false));

    // cap-std hands back its own `File` wrapper; convert into the standard
    // `std::fs::File` so the handle map and per-handle Read/Write/Seek paths
    // below stay untouched. The capability check already happened during
    // `open_with` — once the std file is out, normal syscalls apply.
    let cap_file = dir
        .open_with(&rel, &opts)
        .map_err(|e| format!("fs.open({path_str}): {e}"))?;
    let file = cap_file.into_std();

    Ok(json!({ "id": register(file, owner::current())? }))
}

/// Register `file` for `owner`. A call that was still running when its webview
/// committed a new document registers after `release_owner` has scanned the map;
/// the handle is then closed again instead of being left to the next document.
fn register(file: File, owner: Option<Owner>) -> Result<u64, String> {
    let id = NEXT_HANDLE_ID.fetch_add(1, Ordering::Relaxed);
    let captured = owner.clone();
    HANDLES.insert(
        id,
        Handle {
            owner,
            file: Arc::new(Mutex::new(file)),
        },
    );
    // Checked once the handle is visible: a release that ran before the insert
    // has already bumped the generation by now, and one that runs later finds it.
    if captured.is_some_and(|owner| !owner.is_current()) {
        HANDLES.remove(&id);
        return Err("fs.open: the document that requested it is gone".to_owned());
    }
    Ok(id)
}

/// Look up the handle, clone the Arc, drop the outer lock, then perform
/// the I/O under the per-handle mutex. Blocking syscalls never hold the
/// map lock.
fn with_handle<R>(args: &Value, f: impl FnOnce(&mut File) -> R) -> Result<R, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs: missing 'id'".to_string())?;
    let file = HANDLES
        .get(&id)
        .filter(|entry| entry.permitted())
        .map(|entry| Arc::clone(&entry.file))
        .ok_or_else(|| format!("fs: handle {id} not open"))?;
    let mut guard = file.lock();
    Ok(f(&mut guard))
}

pub(super) fn seek_handle(args: &Value) -> Result<Value, String> {
    let offset = args
        .get("offset")
        .and_then(Value::as_i64)
        .ok_or_else(|| "fs.seek: missing 'offset'".to_string())?;
    let from = match args.get("from").and_then(Value::as_str).unwrap_or("start") {
        "start" => SeekFrom::Start(offset.max(0) as u64),
        "current" => SeekFrom::Current(offset),
        "end" => SeekFrom::End(offset),
        other => {
            return Err(format!(
                "fs.seek: unknown 'from' variant '{other}' — expected 'start', 'current', or 'end'"
            ));
        },
    };
    let new = with_handle(args, |f| f.seek(from))?.map_err(|e| format!("fs.seek: {e}"))?;
    Ok(json!({ "position": new }))
}

// Thread-local scratch buffer for `read_handle`. Re-used across calls on
// the same call-pool worker thread so the per-call `vec![0u8; len]`
// alloc disappears (only the first call on a given thread allocates;
// subsequent calls reuse the same backing storage). Capped at 4 MiB so a
// stray jumbo read doesn't pin a huge allocation per worker thread.
thread_local! {
    static READ_SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}
const READ_SCRATCH_MAX_RETAIN: usize = 4 * 1024 * 1024;

pub(super) fn read_handle(args: &Value) -> Result<Value, String> {
    let len_raw = args
        .get("length")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs.read: missing 'length'".to_string())?;
    if len_raw > MAX_READ_LEN as u64 {
        return Err(IpcError::new(
            "fs",
            "read",
            format!("'length' {len_raw} exceeds maximum {MAX_READ_LEN} bytes"),
        )
        .into_string());
    }
    let len = len_raw as usize;

    READ_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        // Resize-with-zero only fills up to `len`; if the buffer is already
        // ≥ len we just slice — no zero-fill on the hot path.
        if buf.len() < len {
            buf.resize(len, 0);
        }
        let n =
            with_handle(args, |f| f.read(&mut buf[..len]))?.map_err(|e| format!("fs.read: {e}"))?;
        let encoded = STANDARD.encode(&buf[..n]);
        // Drop oversized buffers so a one-off jumbo read doesn't pin
        // megabytes per worker thread for the rest of the process life.
        if buf.capacity() > READ_SCRATCH_MAX_RETAIN {
            buf.shrink_to(READ_SCRATCH_MAX_RETAIN);
        }
        Ok(json!({ "bytes": encoded, "read": n }))
    })
}

pub(super) fn write_handle(args: &Value) -> Result<Value, String> {
    let b64 = args
        .get("bytes")
        .and_then(Value::as_str)
        .ok_or_else(|| "fs.write: missing 'bytes'".to_string())?;
    // Base64 encodes 3 bytes per 4 chars (+ ≤2 chars padding). Bound the
    // encoded string length so `STANDARD.decode` can't allocate beyond
    // `MAX_WRITE_LEN` decoded bytes from a single IPC call.
    const MAX_B64_LEN: usize = MAX_WRITE_LEN.div_ceil(3) * 4 + 4;
    if b64.len() > MAX_B64_LEN {
        return Err(IpcError::new(
            "fs",
            "write",
            format!(
                "'bytes' encoded length {} exceeds maximum {MAX_WRITE_LEN} decoded bytes",
                b64.len()
            ),
        )
        .into_string());
    }
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| format!("fs.write: bad base64 — {e}"))?;
    let n = with_handle(args, |f| f.write(&bytes))?.map_err(|e| format!("fs.write: {e}"))?;
    Ok(json!({ "written": n }))
}

pub(super) fn close_handle(args: &Value) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "fs.close: missing 'id'".to_string())?;
    Ok(Value::Bool(close_id(id)))
}

/// Close handle `id` if the caller may use it; otherwise answer like an
/// unknown id.
fn close_id(id: u64) -> bool {
    HANDLES
        .remove_if(&id, |_, handle| handle.permitted())
        .is_some()
}

/// Close the handles `label` opened.
pub(super) fn release_owner(label: &str) {
    HANDLES.retain(|_, handle| {
        handle
            .owner
            .as_ref()
            .is_none_or(|owner| owner.label() != label)
    });
}

pub(super) fn close_all() {
    HANDLES.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::owner::with_caller;
    use serde_json::json;

    fn temp_file() -> File {
        tempfile::tempfile().expect("temp file")
    }

    fn read_len(id: u64) -> Result<Value, String> {
        read_handle(&json!({ "id": id, "length": 4 }))
    }

    #[test]
    fn handle_of_another_webview_is_indistinguishable_from_an_unknown_id() {
        let id = with_caller("fh-a", || register(temp_file(), owner::current()).unwrap());

        let refused = with_caller("fh-b", || read_len(id)).unwrap_err();
        let unknown = with_caller("fh-b", || read_len(u64::MAX - 1)).unwrap_err();
        assert_eq!(
            refused.replace(&id.to_string(), "N"),
            unknown.replace(&(u64::MAX - 1).to_string(), "N")
        );
        assert!(with_caller("fh-b", || write_handle(
            &json!({ "id": id, "bytes": "AAAA" })
        ))
        .is_err());
        assert!(with_caller("fh-b", || seek_handle(&json!({ "id": id, "offset": 0 }))).is_err());
        assert!(!with_caller("fh-b", || close_id(id)));
        assert!(HANDLES.contains_key(&id));

        assert!(with_caller("fh-a", || read_len(id)).is_ok());
        assert!(with_caller("fh-a", || close_id(id)));
        assert!(!HANDLES.contains_key(&id));
    }

    #[test]
    fn backend_may_use_a_webview_handle() {
        let id = with_caller("fh-c", || register(temp_file(), owner::current()).unwrap());
        assert!(read_len(id).is_ok());
        assert!(close_id(id));
    }

    #[test]
    fn remote_webview_cannot_use_a_backend_handle() {
        use crate::ipc::owner::with_caller_trust;

        let id = register(temp_file(), None).unwrap();
        let refused = with_caller_trust("fh-r", false, || read_len(id)).unwrap_err();
        let unknown = with_caller_trust("fh-r", false, || read_len(u64::MAX - 1)).unwrap_err();
        assert_eq!(
            refused.replace(&id.to_string(), "N"),
            unknown.replace(&(u64::MAX - 1).to_string(), "N")
        );
        assert!(!with_caller_trust("fh-r", false, || close_id(id)));
        assert!(HANDLES.contains_key(&id));
        assert!(with_caller_trust("fh-r", true, || read_len(id)).is_ok());
        assert!(close_id(id));
    }

    #[test]
    fn release_owner_closes_only_that_webviews_handles() {
        let a = with_caller("fh-d", || register(temp_file(), owner::current()).unwrap());
        let b = with_caller("fh-e", || register(temp_file(), owner::current()).unwrap());
        let backend = register(temp_file(), None).unwrap();
        release_owner("fh-d");
        assert!(!HANDLES.contains_key(&a));
        assert!(HANDLES.contains_key(&b));
        assert!(HANDLES.contains_key(&backend));
        close_id(b);
        close_id(backend);
    }

    #[test]
    fn registration_after_the_document_was_replaced_is_released() {
        let label = "fh-late";
        // The create call is received (identity captured) before the commit.
        let late = with_caller(label, owner::current).unwrap();
        owner::bump_generation(label);
        assert!(register(temp_file(), Some(late)).is_err());
        assert!(!HANDLES.iter().any(|handle| handle
            .owner
            .as_ref()
            .is_some_and(|owner| owner.label() == label)));
    }

    #[test]
    fn remote_iframe_cannot_use_the_local_pages_handle() {
        let id = with_caller("fh-frame", || {
            register(temp_file(), owner::current()).unwrap()
        });
        assert!(owner::with_caller_trust("fh-frame", false, || read_len(id)).is_err());
        assert!(!owner::with_caller_trust("fh-frame", false, || close_id(
            id
        )));
        assert!(HANDLES.contains_key(&id));
        assert!(with_caller("fh-frame", || read_len(id)).is_ok());
        assert!(with_caller("fh-frame", || close_id(id)));
    }
}
