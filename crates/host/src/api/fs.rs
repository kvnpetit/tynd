//! `fs` OS API: confined filesystem operations, file handles and watchers.

use parking_lot::{RwLock, RwLockReadGuard};
use serde_json::Value;

use crate::api::HostApi;
use crate::ipc::error::IpcError;

mod handles;
mod ops;
mod watch;

mod roots {
    pub(crate) use crate::security::path_scope::{absolute_lexical, resolve};
}

// Re-exported because `scheme_bin.rs` reaches it through `crate::api::fs::resolve`.
pub(crate) use crate::security::path_scope::resolve;

static SHUTTING_DOWN: RwLock<bool> = RwLock::new(false);

fn admit_dispatch() -> Result<RwLockReadGuard<'static, bool>, String> {
    let admission = SHUTTING_DOWN.read();
    if *admission {
        return Err("fs: host is shutting down; dispatch rejected".into());
    }
    Ok(admission)
}

/// Stop the watches and close the file handles `label` opened.
pub(crate) fn release_webview(label: &str) {
    watch::release_owner(label);
    handles::release_owner(label);
}

/// Close dispatch admission, unregister every watcher, and drop all open file
/// handles. The write lock waits for in-flight dispatches, preventing a handle
/// from appearing after the cleanup snapshot.
pub(crate) fn shutdown() {
    let mut shutting_down = SHUTTING_DOWN.write();
    *shutting_down = true;
    watch::shutdown_all();
    handles::close_all();
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("fs", |method, args| dispatch(method, &args))
};

/// IPC entry point for the `fs` API. Routes to `ops::*` (sync ops),
/// `handles::*` (open/seek/read/write/close state machine), or
/// `watch::*` (fs events). All path args go through `roots::resolve`
/// for cap-std confinement.
///
/// # Errors
/// `"fs.<method>: <message>"`. Frontend handlers parse the prefix.
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    let _admission = admit_dispatch()?;
    match method {
        "readText" => ops::read_text(args),
        "writeText" => ops::write_text(args),
        "exists" => ops::exists(args),
        "stat" => ops::stat(args),
        "readDir" => ops::read_dir(args),
        "mkdir" => ops::mkdir(args),
        "remove" => ops::remove(args),
        "rename" => ops::rename(args),
        "copy" => ops::copy(args),
        "copyDir" => ops::copy_dir(args),
        "trash" => ops::trash_path(args),
        "symlink" => ops::symlink(args),
        "readlink" => ops::readlink(args),
        "hardlink" => ops::hardlink(args),
        "open" => handles::open_handle(args),
        "seek" => handles::seek_handle(args),
        "read" => handles::read_handle(args),
        "write" => handles::write_handle(args),
        "close" => handles::close_handle(args),
        "watch" => watch::watch(args),
        "unwatch" => watch::unwatch(args),
        // readBinary / writeBinary route through `tynd-bin://` for zero-copy.
        _ => Err(IpcError::unknown_method("fs", method).into_string()),
    }
}
