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

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Verifier, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use super::events;

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .build()
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "check" => check(args),
        "downloadAndVerify" => download_and_verify(args),
        "install" => install(args),
        _ => Err(format!("updater.{method}: unknown method")),
    }
}

/// Fetch the manifest, pick the entry for the current platform, compare the
/// version against the caller's current version, and return either
/// `{ available: false }` or the full update metadata.
fn check(args: &Value) -> Result<Value, String> {
    let endpoint = args
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.check: missing 'endpoint'".to_string())?;
    let current = args
        .get("currentVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.check: missing 'currentVersion'".to_string())?;

    let agent = http_agent();
    let resp = agent
        .get(endpoint)
        .call()
        .map_err(|e| format!("updater.check({endpoint}): {e}"))?;
    let mut body = String::new();
    resp.into_reader()
        .take(8 * 1024 * 1024) // 8 MB cap — manifests should be tiny
        .read_to_string(&mut body)
        .map_err(|e| format!("updater.check: read manifest: {e}"))?;
    let manifest: Value =
        serde_json::from_str(&body).map_err(|e| format!("updater.check: manifest parse: {e}"))?;

    let available_version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.check: manifest missing 'version'".to_string())?
        .to_string();

    if !is_strictly_newer(current, &available_version) {
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

/// Download the file at `url` to a temp path, emit throttled
/// `updater:progress { phase, loaded, total, id? }` events, and verify the
/// Ed25519 signature over the downloaded bytes before returning.
fn download_and_verify(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.downloadAndVerify: missing 'url'".to_string())?;
    let signature_b64 = args
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.downloadAndVerify: missing 'signature'".to_string())?;
    let pub_key_b64 = args
        .get("pubKey")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.downloadAndVerify: missing 'pubKey'".to_string())?;
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);

    let signature = decode_signature(signature_b64)?;
    let verifying_key = decode_pub_key(pub_key_b64)?;

    let agent = http_agent();
    let resp = agent
        .get(url)
        .call()
        .map_err(|e| format!("updater.downloadAndVerify({url}): {e}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());

    // Buffer to RAM while downloading — Ed25519 verify needs the full message
    // in one slice (no prehash variant). We only persist to disk AFTER verify
    // succeeds so a failed/tampered download leaves no artifact behind.
    let mut emitter = Progress::new(progress_id.clone(), "download", total);
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

    // Ed25519 verify against the whole file — any byte flip invalidates.
    verifying_key
        .verify(&bytes_accum, &signature)
        .map_err(|e| format!("updater.downloadAndVerify: signature check failed: {e}"))?;

    // Only now is it safe to write: a failed verify above produces no temp
    // file for a caller to accidentally `install()` anyway.
    let tmp = tempfile::Builder::new()
        .prefix("tynd-update-")
        .tempfile()
        .map_err(|e| format!("updater.downloadAndVerify: tempfile: {e}"))?;
    let path = tmp.path().to_path_buf();
    let (mut file, _persist_guard) = tmp.keep().map_err(|e| format!("tempfile keep: {e}"))?;
    file.write_all(&bytes_accum)
        .map_err(|e| format!("updater.downloadAndVerify: write: {e}"))?;

    emitter.emit_phase("verified");

    Ok(json!({
        "path": path.to_string_lossy(),
        "size": bytes_accum.len(),
    }))
}

/// tao/wry report OS on their own axis; this mirrors the identifiers the
/// GitHub Releases attestations use so manifests stay single-source.
fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other, // windows / linux already match
    };
    let arch = std::env::consts::ARCH; // x86_64 / aarch64 / …
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

/// Compare dotted integer triples `X.Y.Z`. Returns true iff `available`
/// sorts strictly after `current`. Pre-release suffixes (`-beta.1`) on
/// the patch part are trimmed before parsing.
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

/// Inline throttled emitter for `updater:progress`. Mirrors the shape of
/// `http:progress` but uses its own event name so subscribers can scope.
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

/// Swap the downloaded artifact for the currently running binary, then
/// (by default) relaunch from the new version and exit the current process.
///
/// The running exe holds a lock on its own file on Windows, so we cannot
/// just `fs::rename`. Platform strategies:
/// - **Windows**: spawn `cmd.exe` with a short timeout + `move /y` + `start`.
///   The delay lets the current process exit so the .exe unlocks; then the
///   move succeeds and cmd relaunches the new binary.
/// - **Linux AppImage** (and other single-file binaries): `fs::rename` is
///   safe while the exe is running — ELF is fully loaded at exec time, the
///   kernel keeps a private mapping of the old inode. Chmod +x defensively.
/// - **macOS**: `.app` bundles are directories, updates ship as archives —
///   not yet supported. Callers are expected to handle the swap themselves
///   and just call `relaunch(false)`-style exit, which we don't expose yet.
fn install(args: &Value) -> Result<Value, String> {
    let new_path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "updater.install: missing 'path'".to_string())?;
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
        Err(
            "updater.install: macOS .app swap not yet implemented — handle the swap + \
             relaunch in userland and call process.exit()."
                .to_string(),
        )
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
        .ok_or_else(|| "updater.install: current_exe path is not UTF-8".to_string())?;

    // Defense-in-depth: the cmd script string-interpolates both paths. tempfile
    // names are normally safe, but an attacker-controlled path containing any
    // of `" & | ^ > < %` could break out of the quoted token. Reject upfront
    // instead of trying to escape every cmd.exe quirk.
    for (label, p) in [("new_path", new_path), ("current", current_str)] {
        if p.chars()
            .any(|c| matches!(c, '"' | '&' | '|' | '^' | '>' | '<' | '%' | '\n' | '\r'))
        {
            return Err(format!(
                "updater.install: {label} contains shell metacharacters, refusing"
            ));
        }
    }

    // cmd escape: wrap paths in "…"; cmd itself reads %1/%2 via start-quoting.
    // `start ""` first arg is the window title; omitting it makes cmd treat the
    // next quoted token as the title, so the empty `""` placeholder is load-bearing.
    let launch_tail = if relaunch {
        format!(" & start \"\" \"{current_str}\"")
    } else {
        String::new()
    };
    let script = format!(
        "timeout /t 2 /nobreak > nul & move /y \"{new_path}\" \"{current_str}\"{launch_tail}"
    );

    std::process::Command::new("cmd")
        .args(["/c", &script])
        .spawn()
        .map_err(|e| format!("updater.install: spawn cmd: {e}"))?;

    if relaunch {
        // Give the new process a tick to take over before we exit.
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

    std::fs::rename(new_path, current)
        .map_err(|e| format!("updater.install: rename to {}: {e}", current.display()))?;
    // Make sure the replacement keeps the +x bit even if the download lost it.
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
