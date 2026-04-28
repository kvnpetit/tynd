//! Auto-updater — manifest check + signed binary download.
//!
//! **Trust model.** The manifest itself is not signed. Each platform entry
//! carries an Ed25519 signature over the raw bytes of the downloaded file.
//! Server tampering with the manifest can at worst redirect to a different
//! URL — the signature still has to verify against the pub-key baked into
//! the app binary, so a mismatched payload is rejected.
//!
//! Manifest format (Tauri-compatible enough for cross-port):
//! ```json
//! {
//!   "version": "1.2.3",
//!   "notes": "release notes...",
//!   "pub_date": "2026-04-19T12:00:00Z",
//!   "platforms": {
//!     "windows-x86_64": { "url": "...", "signature": "<base64 Ed25519>" },
//!     "darwin-aarch64": { "url": "...", "signature": "<base64 Ed25519>" }
//!   }
//! }
//! ```

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::events;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "check" => check(args),
        "downloadAndVerify" => download_and_verify(args),
        "install" => install(args),
        "startPeriodicCheck" => start_periodic_check(args),
        "stopPeriodicCheck" => {
            stop_periodic_check();
            Ok(Value::Null)
        },
        _ => Err(format!("updater.{method}: unknown method")),
    }
}

/// Running periodic check, if any. Replacing / clearing the slot signals
/// the worker thread to stop and joins it.
struct PeriodicTask {
    running: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl PeriodicTask {
    fn stop(mut self) {
        self.running.store(false, Ordering::Release);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}

static PERIODIC: OnceLock<Mutex<Option<PeriodicTask>>> = OnceLock::new();
static PERIODIC_ID: AtomicU64 = AtomicU64::new(0);

fn periodic_slot() -> &'static Mutex<Option<PeriodicTask>> {
    PERIODIC.get_or_init(|| Mutex::new(None))
}

fn start_periodic_check(args: &Value) -> Result<Value, String> {
    let mut slot = periodic_slot().lock();
    if slot.is_some() {
        // Already running — return the current task id unchanged.
        return Ok(json!({ "id": PERIODIC_ID.load(Ordering::Relaxed) }));
    }

    let id = PERIODIC_ID.fetch_add(1, Ordering::Relaxed) + 1;
    // 1000 ms floor keeps a typo'd interval from burning a thread at 100%.
    let interval = Duration::from_millis(
        args.get("intervalMs")
            .and_then(Value::as_u64)
            .unwrap_or(60 * 60 * 1000)
            .max(1000),
    );
    let args_owned = args.clone();
    let running = Arc::new(AtomicBool::new(true));
    let r = Arc::clone(&running);

    // 250ms tick lets `stop()` take effect in bounded time even when the
    // configured interval is long.
    let join = std::thread::Builder::new()
        .name(format!("updater-check-{id}"))
        .spawn(move || {
            let tick = Duration::from_millis(250);
            while r.load(Ordering::Acquire) {
                match check(&args_owned) {
                    Ok(info) => events::emit(
                        "updater:check",
                        &json!({ "ok": true, "info": info, "id": id }),
                    ),
                    Err(err) => events::emit(
                        "updater:check",
                        &json!({ "ok": false, "error": err, "id": id }),
                    ),
                }
                let mut slept = Duration::ZERO;
                while slept < interval && r.load(Ordering::Acquire) {
                    let step = tick.min(interval - slept);
                    std::thread::sleep(step);
                    slept += step;
                }
            }
        })
        .ok();

    *slot = Some(PeriodicTask { running, join });
    Ok(json!({ "id": id }))
}

fn stop_periodic_check() {
    let prev = periodic_slot().lock().take();
    if let Some(task) = prev {
        task.stop();
    }
}

fn http_agent(proxy: Option<&str>) -> Result<ureq::Agent, String> {
    let mut b = ureq::AgentBuilder::new().timeout(Duration::from_secs(60));
    if let Some(url) = proxy {
        let p = ureq::Proxy::new(url).map_err(|e| format!("updater: bad proxy URL: {e}"))?;
        b = b.proxy(p);
    }
    Ok(b.build())
}

fn apply_headers(mut req: ureq::Request, headers: Option<&Value>) -> ureq::Request {
    if let Some(obj) = headers.and_then(Value::as_object) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                req = req.set(k, s);
            }
        }
    }
    req
}

fn check(args: &Value) -> Result<Value, String> {
    let endpoint = args
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or("updater.check: missing 'endpoint'")?;
    let current = args
        .get("currentVersion")
        .and_then(Value::as_str)
        .ok_or("updater.check: missing 'currentVersion'")?;
    let proxy = args.get("proxy").and_then(Value::as_str);
    let allow_downgrade = args
        .get("allowDowngrade")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let agent = http_agent(proxy)?;
    let req = apply_headers(agent.get(endpoint), args.get("headers"));
    let resp = req
        .call()
        .map_err(|e| format!("updater.check({endpoint}): {e}"))?;
    let mut body = String::new();
    resp.into_reader()
        .take(8 * 1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|e| format!("updater.check: read manifest: {e}"))?;
    let manifest: Value =
        serde_json::from_str(&body).map_err(|e| format!("updater.check: manifest parse: {e}"))?;

    let available_version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("updater.check: manifest missing 'version'")?
        .to_string();

    if !allow_downgrade && !is_strictly_newer(current, &available_version) {
        return Ok(json!({ "available": false }));
    }

    let key = platform_key();
    let entry = manifest
        .get("platforms")
        .and_then(|p| p.get(&key))
        .ok_or_else(|| format!("updater.check: no platform entry for '{key}'"))?;

    let url = entry
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("updater.check: platform '{key}' missing 'url'"))?;
    let signature = entry
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("updater.check: platform '{key}' missing 'signature'"))?;

    Ok(json!({
        "available": true,
        "version": available_version,
        "notes": manifest.get("notes").cloned().unwrap_or(Value::Null),
        "pubDate": manifest.get("pub_date").cloned().unwrap_or(Value::Null),
        "url": url,
        "signature": signature,
        "platform": key,
    }))
}

fn download_and_verify(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndVerify: missing 'url'")?;
    let signature_b64 = args
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndVerify: missing 'signature'")?;
    let pub_key_b64 = args
        .get("pubKey")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndVerify: missing 'pubKey'")?;
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);

    let signature = decode_signature(signature_b64)?;
    let verifying_key = decode_pub_key(pub_key_b64)?;
    let proxy = args.get("proxy").and_then(Value::as_str);

    let agent = http_agent(proxy)?;
    let req = apply_headers(agent.get(url), args.get("headers"));
    let resp = req
        .call()
        .map_err(|e| format!("updater.downloadAndVerify({url}): {e}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());

    let mut emitter = Progress::new(progress_id, "download", total);
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut bytes_accum: Vec<u8> = Vec::with_capacity(total.unwrap_or(0) as usize);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("updater.downloadAndVerify: read: {e}"))?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        bytes_accum.extend_from_slice(&buf[..n]);
        emitter.advance(n, false);
    }

    verifying_key
        .verify(&bytes_accum, &signature)
        .map_err(|e| format!("updater.downloadAndVerify: signature check failed: {e}"))?;

    let tmp = std::env::temp_dir().join(format!("tynd-update-{}", simple_rand()));
    std::fs::write(&tmp, &bytes_accum)
        .map_err(|e| format!("updater.downloadAndVerify: write: {e}"))?;

    emitter.emit_phase("verified");

    Ok(json!({
        "path": tmp.to_string_lossy(),
        "size": bytes_accum.len(),
    }))
}

fn simple_rand() -> String {
    // Small, non-crypto unique suffix. PID + full nanos + atomic counter so
    // two parallel calls in the same process can't collide.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = u64::from(std::process::id());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{pid:x}-{nanos:x}-{n:x}")
}

fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = std::env::consts::ARCH;
    format!("{os}-{arch}")
}

fn decode_signature(b64: &str) -> Result<Signature, String> {
    let raw = STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("signature base64: {e}"))?;
    if raw.len() != SIGNATURE_LENGTH {
        return Err(format!(
            "signature length: expected {SIGNATURE_LENGTH}, got {}",
            raw.len()
        ));
    }
    let bytes: [u8; SIGNATURE_LENGTH] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "signature: bad length".to_string())?;
    Ok(Signature::from_bytes(&bytes))
}

fn decode_pub_key(b64: &str) -> Result<VerifyingKey, String> {
    let raw = STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("pubKey base64: {e}"))?;
    if raw.len() != PUBLIC_KEY_LENGTH {
        return Err(format!(
            "pubKey length: expected {PUBLIC_KEY_LENGTH}, got {}",
            raw.len()
        ));
    }
    let bytes: [u8; PUBLIC_KEY_LENGTH] = raw
        .as_slice()
        .try_into()
        .map_err(|_| "pubKey: bad length".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| format!("pubKey: {e}"))
}

fn is_strictly_newer(current: &str, available: &str) -> bool {
    let Some(c) = parse_triple(current) else {
        return true;
    };
    let Some(a) = parse_triple(available) else {
        return false;
    };
    a > c
}

fn parse_triple(s: &str) -> Option<(u32, u32, u32)> {
    let v = s.trim().trim_start_matches('v');
    let mut it = v.split('.');
    let major: u32 = it.next()?.parse().ok()?;
    let minor: u32 = it.next()?.parse().ok()?;
    let patch_raw = it.next()?;
    let patch_digits: String = patch_raw.chars().take_while(char::is_ascii_digit).collect();
    let patch: u32 = patch_digits.parse().ok()?;
    Some((major, minor, patch))
}

struct Progress {
    id: Option<String>,
    phase: &'static str,
    total: Option<u64>,
    loaded: u64,
    last_emit: Instant,
}

impl Progress {
    fn new(id: Option<String>, phase: &'static str, total: Option<u64>) -> Self {
        Self {
            id,
            phase,
            total,
            loaded: 0,
            last_emit: Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
        }
    }

    fn advance(&mut self, delta: usize, force: bool) {
        self.loaded += delta as u64;
        if !force && self.last_emit.elapsed() < Duration::from_millis(50) {
            return;
        }
        self.last_emit = Instant::now();
        self.emit();
    }

    fn emit_phase(&mut self, phase: &'static str) {
        self.phase = phase;
        self.emit();
    }

    fn emit(&self) {
        events::emit(
            "updater:progress",
            &json!({
                "id": self.id,
                "phase": self.phase,
                "loaded": self.loaded,
                "total": self.total,
            }),
        );
    }
}

fn install(args: &Value) -> Result<Value, String> {
    let new_path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("updater.install: missing 'path'")?;
    let relaunch = args
        .get("relaunch")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let current =
        std::env::current_exe().map_err(|e| format!("updater.install: current_exe: {e}"))?;

    if !std::path::Path::new(new_path).exists() {
        return Err(format!(
            "updater.install: downloaded file not found at '{new_path}'"
        ));
    }

    #[cfg(target_os = "windows")]
    return install_windows(new_path, &current, relaunch);

    #[cfg(target_os = "linux")]
    return install_linux(new_path, &current, relaunch);

    #[cfg(target_os = "macos")]
    {
        let _ = (new_path, current, relaunch);
        Err("updater.install: macOS .app swap not yet implemented".to_string())
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    Err("updater.install: unsupported target OS".to_string())
}

#[cfg(target_os = "windows")]
fn install_windows(
    new_path: &str,
    current: &std::path::Path,
    relaunch: bool,
) -> Result<Value, String> {
    let current_str = current
        .to_str()
        .ok_or("updater.install: current_exe path is not UTF-8")?;

    // `self-replace` handles the Windows file-lock dance — moves the
    // running .exe out of the way (rename) and atomically swaps the new
    // binary in. Replaces the previous `cmd /c move` script which had
    // to dodge cmd metacharacter injection by hand.
    self_replace::self_replace(new_path)
        .map_err(|e| format!("updater.install: self_replace: {e}"))?;

    // Best-effort cleanup of the staged download — `self_replace`
    // doesn't move it; it copies. The old name only existed in our
    // download cache so deleting it post-swap is safe.
    let _ = std::fs::remove_file(new_path);

    if relaunch {
        // Spawn the swapped-in current_exe before exiting so the new
        // binary takes over without user re-launch.
        std::process::Command::new(current)
            .spawn()
            .map_err(|e| format!("updater.install: relaunch: {e}"))?;
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        });
    }
    Ok(json!({ "installed": true, "path": current_str, "relaunch": relaunch }))
}

#[cfg(target_os = "linux")]
fn install_linux(
    new_path: &str,
    current: &std::path::Path,
    relaunch: bool,
) -> Result<Value, String> {
    use std::os::unix::fs::PermissionsExt as _;

    // `self_replace` is a no-op on Linux compared to a plain `rename`,
    // but it normalizes the cross-platform code path and protects against
    // future kernel quirks (e.g. exec-busy on overlay filesystems).
    self_replace::self_replace(new_path)
        .map_err(|e| format!("updater.install: self_replace: {e}"))?;
    let _ = std::fs::remove_file(new_path);

    if let Ok(meta) = std::fs::metadata(current) {
        let mut perm = meta.permissions();
        perm.set_mode(perm.mode() | 0o755);
        let _ = std::fs::set_permissions(current, perm);
    }

    if relaunch {
        std::process::Command::new(current)
            .spawn()
            .map_err(|e| format!("updater.install: relaunch: {e}"))?;
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            std::process::exit(0);
        });
    }
    Ok(json!({
        "installed": true,
        "path": current.to_string_lossy(),
        "relaunch": relaunch,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triple_parse_variants() {
        assert_eq!(parse_triple("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_triple("v10.0.11"), Some((10, 0, 11)));
        assert_eq!(parse_triple("  1.2.3  "), Some((1, 2, 3)));
        assert_eq!(parse_triple("1.2.3-beta.4"), Some((1, 2, 3)));
        assert_eq!(parse_triple("1.2"), None);
        assert_eq!(parse_triple("nope"), None);
    }

    #[test]
    fn newer_only_on_strictly_greater() {
        assert!(is_strictly_newer("1.0.0", "1.0.1"));
        assert!(is_strictly_newer("1.0.0", "1.1.0"));
        assert!(is_strictly_newer("1.0.0", "2.0.0"));
        assert!(!is_strictly_newer("1.0.1", "1.0.1"));
        assert!(!is_strictly_newer("1.0.1", "1.0.0"));
    }

    #[test]
    fn platform_key_has_os_and_arch() {
        let k = platform_key();
        assert!(k.contains('-'));
        assert!(matches!(
            k.split('-').next(),
            Some("windows" | "darwin" | "linux")
        ));
    }
}
