//! Opt-in JSON-RPC debug protocol over UDS, named pipe, or TCP loopback.

pub(crate) mod methods;
mod webview_glue;

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock, OnceLock,
    },
};

use crossbeam_channel::Sender;
use dashmap::DashMap;
pub(crate) use methods::window::set_hook as set_window_hook;
use serde_json::{json, Value};
pub(crate) use tynd_debug_protocol::{Endpoint, IpcFrame};
use tynd_protocol::runtime::{BackendCall, BackendReturnPayload};
pub(crate) use webview_glue::{set_eval_hook, DEBUG_HELPER_JS};

static STARTED: AtomicBool = AtomicBool::new(false);
static DISCOVERY_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Bearer token required by the initial debug `hello` RPC.
static AUTH_TOKEN: OnceLock<String> = OnceLock::new();
/// A packaged app serves the debug protocol only when its pack was built with
/// the debug flag; an unpacked run (`pack` is `None`) always may.
const fn start_allowed(pack: Option<bool>) -> bool {
    match pack {
        Some(allowed) => allowed,
        None => true,
    }
}

pub(crate) use tynd_debug_protocol::ipc_tap::is_installed as ipc_tap_enabled;

type CapturedRequest = (String, Value, Option<String>, u64);
static IPC_REQUESTS: parking_lot::Mutex<VecDeque<CapturedRequest>> =
    parking_lot::Mutex::new(VecDeque::new());
static IPC_REPLAY_TX: OnceLock<Sender<BackendCall>> = OnceLock::new();
type ReplayReply = Sender<Result<Value, String>>;
static IPC_REPLAYS: LazyLock<DashMap<String, ReplayReply>> = LazyLock::new(DashMap::new);
const IPC_REPLAY_CAPACITY: usize = 256;
pub(crate) const IPC_CAPTURE_MAX_BYTES: usize = 64 * 1024;
static IPC_REPLAY_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn install_ipc_replay(call_tx: Sender<BackendCall>) {
    let _ = IPC_REPLAY_TX.set(call_tx);
}

pub(crate) fn record_ipc_request(id: &str, method: Option<&str>, args: Value) {
    let ts = tynd_debug_protocol::ipc_tap::now_ms();
    let method = method.map(|value| {
        if value.len() <= 256 {
            value.to_owned()
        } else {
            "<method-name-truncated>".to_owned()
        }
    });
    {
        let mut requests = IPC_REQUESTS.lock();
        if requests.len() == IPC_REPLAY_CAPACITY {
            requests.pop_front();
        }
        requests.push_back((id.to_owned(), args.clone(), method.clone(), ts));
    }
    tynd_debug_protocol::ipc_tap::record(IpcFrame {
        ts,
        direction: "in",
        call_id: Some(id.to_owned()),
        method,
        args,
        result: Value::Null,
        duration_ms: None,
    });
}

pub(crate) fn replay_ipc(id: &str) -> Result<Value, String> {
    let mut request = IPC_REQUESTS
        .lock()
        .iter()
        .find_map(|(request_id, value, _, _)| (request_id == id).then(|| value.clone()))
        .ok_or_else(|| format!("IPC frame '{id}' is no longer in the replay buffer"))?;
    if request
        .get("__tynd_capture_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(format!(
            "IPC frame '{id}' exceeded the {IPC_CAPTURE_MAX_BYTES}-byte replay limit"
        ));
    }
    let replay_id = format!(
        "debug-replay-{}-{}",
        std::process::id(),
        IPC_REPLAY_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    request
        .as_object_mut()
        .ok_or_else(|| "captured IPC request is not an object".to_string())?
        .insert("id".into(), Value::String(replay_id.clone()));
    let encoded = serde_json::to_string(&request)
        .map_err(|error| format!("encode replay request: {error}"))?;
    let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
    IPC_REPLAYS.insert(replay_id.clone(), reply_tx);
    let Some(call_tx) = IPC_REPLAY_TX.get() else {
        IPC_REPLAYS.remove(&replay_id);
        return Err("IPC replay bridge not installed".into());
    };
    if let Err(error) = call_tx.send(BackendCall::Raw(encoded)) {
        IPC_REPLAYS.remove(&replay_id);
        return Err(format!("send replay request: {error}"));
    }
    reply_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .map_err(|error| {
            IPC_REPLAYS.remove(&replay_id);
            format!("IPC replay timeout: {error}")
        })?
}

pub(crate) fn complete_ipc_replay(id: &str, payload: &BackendReturnPayload) -> bool {
    let Some((_, reply)) = IPC_REPLAYS.remove(id) else {
        return false;
    };
    let result = match payload {
        BackendReturnPayload::Ok(value) => {
            Ok(serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.clone())))
        },
        BackendReturnPayload::Err(error) => Err(error.clone()),
    };
    let _ = reply.send(result);
    true
}

pub(crate) fn record_ipc_response(id: &str, ok: bool, payload: &str) {
    if !ipc_tap_enabled() {
        return;
    }
    let now = tynd_debug_protocol::ipc_tap::now_ms();
    let (method, started) = IPC_REQUESTS
        .lock()
        .iter()
        .rev()
        .find_map(|(request_id, _, method, started)| {
            (request_id == id).then(|| (method.clone(), *started))
        })
        .unwrap_or((None, now));
    let decoded = if payload.len() <= IPC_CAPTURE_MAX_BYTES {
        serde_json::from_str(payload).unwrap_or_else(|_| Value::String(payload.to_owned()))
    } else {
        json!({
            "__tynd_capture_truncated": true,
            "bytes": payload.len(),
        })
    };
    let result = if ok {
        decoded
    } else {
        let message = if payload.len() > IPC_CAPTURE_MAX_BYTES {
            format!("runtime error payload exceeded the {IPC_CAPTURE_MAX_BYTES}-byte capture limit")
        } else {
            decoded
                .as_str()
                .unwrap_or("runtime returned a non-string error")
                .to_owned()
        };
        json!({
            "error": {
                "code": "RUNTIME_ERROR",
                "message": message,
            }
        })
    };
    tynd_debug_protocol::ipc_tap::record(IpcFrame {
        ts: now,
        direction: "out",
        call_id: Some(id.to_owned()),
        method,
        args: Value::Null,
        result,
        duration_ms: Some(now.saturating_sub(started)),
    });
}

/// Best-effort cleanup for the debug discovery file.
pub(crate) fn cleanup_discovery_file() {
    if let Some(path) = DISCOVERY_PATH.get() {
        let _ = std::fs::remove_file(path);
    }
}

/// Start the debug server when `TYND_DEBUG_SOCKET` or `TYND_DEBUG_PORT` is set.
pub(crate) fn start_if_enabled() -> Option<Endpoint> {
    if STARTED.swap(true, Ordering::AcqRel) {
        return None;
    }

    let endpoint = read_endpoint_from_env()?;

    activate(endpoint)
}

/// Force activation against a specific endpoint.
pub(crate) fn activate(endpoint: Endpoint) -> Option<Endpoint> {
    if !start_allowed(crate::embed::pack_debug_policy()) {
        tracing::warn!(
            target: "tynd::debug",
            "debug protocol requested but refused: this packaged app was not built with `tynd build --debug-protocol`"
        );
        return None;
    }
    if AUTH_TOKEN.get().is_none() {
        let token = match generate_auth_token() {
            Ok(token) => token,
            Err(error) => {
                tracing::error!(target: "tynd::debug", "debug auth token generation failed: {error}");
                return None;
            },
        };
        let _ = AUTH_TOKEN.set(token);
    }

    tynd_debug_protocol::ipc_tap::install();
    tynd_debug_protocol::log_tap::install();

    let router: tynd_debug_protocol::MethodRouter = Arc::new(methods::dispatch);
    if let Err(e) = tynd_debug_protocol::server::spawn_accept_loop(
        endpoint.clone(),
        AUTH_TOKEN.get().cloned(),
        router,
    ) {
        tracing::error!(target: "tynd::debug", "debug server bind failed: {e}");
        return None;
    }

    if let Err(e) = write_discovery_file(&endpoint) {
        tracing::error!(target: "tynd::debug", "debug protocol disabled: discovery file write failed: {e}");
        return None;
    }
    tracing::info!(target: "tynd::debug", "debug protocol listening on {:?}", endpoint);
    Some(endpoint)
}

fn read_endpoint_from_env() -> Option<Endpoint> {
    if let Some(socket) = std::env::var_os("TYND_DEBUG_SOCKET") {
        return Some(Endpoint::Unix(PathBuf::from(socket)));
    }
    if let Ok(port) = std::env::var("TYND_DEBUG_PORT") {
        if let Ok(p) = port.parse::<u16>() {
            return Some(Endpoint::Tcp(p));
        }
    }
    None
}

fn write_discovery_file(endpoint: &Endpoint) -> std::io::Result<()> {
    let dir = discovery_dir()?;
    ensure_discovery_dir(&dir)?;
    purge_stale(&dir);
    let pid = std::process::id();
    let path = dir.join(format!("{pid}.json"));
    let endpoint_json = match endpoint {
        Endpoint::Unix(p) => serde_json::json!({ "unix": p.to_string_lossy() }),
        Endpoint::Tcp(port) => serde_json::json!({ "tcp": port }),
    };
    let payload = serde_json::json!({
        "pid": pid,
        "appId": crate::api::app::AppApi::get_name(),
        "appVersion": crate::api::app::AppApi::get_version(),
        "endpoint": endpoint_json,
        "startedAt": now_iso8601(),
        "cookie": AUTH_TOKEN.get().cloned().unwrap_or_default(),
    });
    let body = serde_json::to_vec_pretty(&payload).unwrap_or_default();
    #[cfg(unix)]
    {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(&body)?;
    }

    #[cfg(windows)]
    {
        std::fs::write(&path, &body)?;
        if let Err(e) = restrict_dacl_to_owner(&path) {
            let _ = std::fs::remove_file(&path);
            return Err(std::io::Error::new(
                e.kind(),
                format!("refusing insecure debug discovery file: {e}"),
            ));
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        std::fs::write(&path, &body)?;
    }
    let _ = DISCOVERY_PATH.set(path);
    Ok(())
}

#[cfg(unix)]
fn ensure_discovery_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::symlink_metadata(dir) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "debug discovery path must be a real directory",
            ));
        },
        Ok(_) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700).create(dir)?;
        },
        Err(error) => return Err(error),
    }

    let metadata = std::fs::symlink_metadata(dir)?;
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "debug discovery directory is not owned by the current user",
        ));
    }
    if metadata.mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }

    let secured = std::fs::symlink_metadata(dir)?;
    if secured.file_type().is_symlink()
        || !secured.is_dir()
        || secured.uid() != unsafe { libc::geteuid() }
        || secured.mode() & 0o777 != 0o700
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "debug discovery directory could not be secured to owner-only access",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_discovery_dir(dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(windows)]
/// Restrict the token-bearing discovery file to the current owner.
fn restrict_dacl_to_owner(path: &std::path::Path) -> std::io::Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};

    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
                SE_FILE_OBJECT,
            },
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        },
    };

    let sddl_w: Vec<u16> = "D:PAI(A;;FA;;;OW)\0".encode_utf16().collect();
    let mut sd_ptr: PSECURITY_DESCRIPTOR = ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_w.as_ptr(),
            1, // SDDL_REVISION_1
            &raw mut sd_ptr,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut dacl_present: i32 = 0;
    let mut dacl_ptr: *mut windows_sys::Win32::Security::ACL = ptr::null_mut();
    let mut dacl_defaulted: i32 = 0;
    let got = unsafe {
        windows_sys::Win32::Security::GetSecurityDescriptorDacl(
            sd_ptr,
            &raw mut dacl_present,
            &raw mut dacl_ptr,
            &raw mut dacl_defaulted,
        )
    };
    if got == 0 || dacl_present == 0 {
        let err = std::io::Error::last_os_error();
        unsafe {
            LocalFree(sd_ptr.cast());
        }
        return Err(err);
    }

    let mut path_w: Vec<u16> = path.as_os_str().encode_wide().collect();
    path_w.push(0);

    let status = unsafe {
        SetNamedSecurityInfoW(
            path_w.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            dacl_ptr,
            ptr::null_mut(),
        )
    };
    unsafe {
        LocalFree(sd_ptr.cast());
    }
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(i32::from_ne_bytes(
            status.to_ne_bytes(),
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn discovery_dir() -> std::io::Result<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(d).join("tynd"));
    }
    Ok(PathBuf::from(format!("/tmp/tynd-{}", unsafe {
        libc::geteuid()
    })))
}

#[cfg(target_os = "macos")]
fn discovery_dir() -> std::io::Result<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "home dir not found"))?;
    Ok(home.join("Library/Application Support/tynd/debug"))
}

#[cfg(target_os = "windows")]
fn discovery_dir() -> std::io::Result<PathBuf> {
    let base = dirs::data_local_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA not found")
    })?;
    Ok(base.join("tynd").join("debug"))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn discovery_dir() -> std::io::Result<PathBuf> {
    Ok(std::env::temp_dir().join("tynd"))
}

/// Drop any stale discovery files whose pid is no longer alive. Best-effort.
fn purge_stale(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(pid) = stem.parse::<u32>() else {
            continue;
        };
        if !pid_alive(pid) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    // `kill -0` returns 0 if the pid is alive, ESRCH otherwise.
    let result = unsafe { libc_kill_zero(pid) };
    result == 0
}

#[cfg(unix)]
extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[cfg(unix)]
unsafe fn libc_kill_zero(pid: i32) -> i32 {
    // SAFETY: signal 0 performs an existence/permission probe only. `pid` is
    // passed by value and no pointer or memory lifetime is involved.
    unsafe { kill(pid, 0) }
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    // OpenProcess succeeds when the pid is live. We don't actually
    // need the handle, just the open/close round-trip.
    type Dword = u32;
    type Handle = *mut std::ffi::c_void;
    extern "system" {
        fn OpenProcess(access: Dword, inherit: i32, pid: Dword) -> Handle;
        fn CloseHandle(h: Handle) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: Dword = 0x1000;
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return false;
    }
    unsafe { CloseHandle(handle) };
    true
}

fn generate_auth_token() -> Result<String, getrandom::Error> {
    generate_auth_token_with(|bytes| getrandom::fill(bytes))
}

fn generate_auth_token_with<E>(
    fill: impl FnOnce(&mut [u8; 32]) -> Result<(), E>,
) -> Result<String, E> {
    let mut buf = [0u8; 32];
    fill(&mut buf)?;
    let mut hex = String::with_capacity(buf.len() * 2);
    for byte in buf {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format_unix_timestamp(secs)
}

fn format_unix_timestamp(secs: u64) -> String {
    let days = (secs / 86_400).cast_signed();
    let seconds_of_day = secs % 86_400;
    // Gregorian civil date from days since Unix epoch. This is the
    // chrono-free Howard Hinnant algorithm, valid for the full u64 range
    // that still fits the formatted year.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::start_allowed;

    #[test]
    fn unpacked_runs_may_start_the_debug_server() {
        assert!(start_allowed(None));
    }

    #[test]
    fn packed_apps_need_the_debug_flag() {
        assert!(!start_allowed(Some(false)));
        assert!(start_allowed(Some(true)));
    }

    #[cfg(unix)]
    #[test]
    fn pid_probe_requires_a_positive_pid_in_range() {
        assert!(!super::pid_alive(0));
        assert!(!super::pid_alive(u32::MAX));
        assert!(super::pid_alive(std::process::id()));
    }
}
