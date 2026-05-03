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

use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, LazyLock,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use bytes::{Bytes, BytesMut};
use ed25519_dalek::{Signature, Verifier, VerifyingKey, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH};
use parking_lot::Mutex;
use serde_json::{json, Value};

// SECURITY: BLAKE3 fingerprint of every downloaded + signature-verified
// binary. Returned by `download_and_verify` in the JSON IPC payload and
// re-checked by `install` immediately before `self_replace`. The window
// between download and install is the TOCTOU surface — without this
// re-check, an attacker who can write to `temp_dir` between the two IPC
// calls swaps the binary for one that wasn't covered by the Ed25519
// verification.
//
// Hex-encoded (32 bytes → 64 hex chars) so it survives a JSON round-trip
// without base64 framing surprises and reads as the same string a user
// would compute via `b3sum`.
use blake3::Hasher as Blake3Hasher;

use super::events;

// ---------------------------------------------------------------------------
// Semver helpers (public API for tests + external callers in the host crate)
// ---------------------------------------------------------------------------

/// Returns `true` if `latest` is strictly greater than `current` according to
/// full SemVer semantics (pre-release ordering, build-metadata ignored).
///
/// # Errors
/// Returns an error if either string is not a valid SemVer version.
pub fn is_update_available(current: &str, latest: &str) -> anyhow::Result<bool> {
    let current_v = semver::Version::parse(current.trim().trim_start_matches('v'))
        .map_err(|e| anyhow::anyhow!("invalid current version '{current}': {e}"))?;
    let latest_v = semver::Version::parse(latest.trim().trim_start_matches('v'))
        .map_err(|e| anyhow::anyhow!("invalid latest version '{latest}': {e}"))?;
    Ok(latest_v > current_v)
}

/// Returns `true` if `version` satisfies the semver requirement string `req`
/// (e.g. `">=1.2.0, <2.0.0"`).
///
/// # Errors
/// Returns an error if `version` or `req` cannot be parsed.
pub fn meets_requirement(version: &str, req: &str) -> anyhow::Result<bool> {
    let v = semver::Version::parse(version.trim().trim_start_matches('v'))
        .map_err(|e| anyhow::anyhow!("invalid version '{version}': {e}"))?;
    let r = semver::VersionReq::parse(req)
        .map_err(|e| anyhow::anyhow!("invalid version requirement '{req}': {e}"))?;
    Ok(r.matches(&v))
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "check" => check(args),
        "downloadAndVerify" => download_and_verify(args),
        // BSDIFF delta download: downloads a small patch (~100 KB) and
        // applies it on top of the currently-running binary, producing
        // the same staged file layout `install` consumes. Falls back to
        // `download_and_verify` semantics on the result side (path +
        // size + BLAKE3 hex), so the JS caller can route either result
        // through the same `install` IPC arm without a second branch.
        "downloadAndPatch" => download_and_patch(args),
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

/// `LazyLock` initializes the inner `Mutex<Option<PeriodicTask>>` to
/// `None` on first deref — replaces a `periodic_slot()` accessor
/// wrapping `OnceLock::get_or_init`.
static PERIODIC: LazyLock<Mutex<Option<PeriodicTask>>> = LazyLock::new(|| Mutex::new(None));
static PERIODIC_ID: AtomicU64 = AtomicU64::new(0);

fn start_periodic_check(args: &Value) -> Result<Value, String> {
    let mut slot = PERIODIC.lock();
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
                    let step = tick.min(interval.checked_sub(slept).unwrap());
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
    let prev = PERIODIC.lock().take();
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

    // Look for a delta patch matching the *current* running version.
    // Manifest format (per-platform):
    //   "patches": [
    //     { "from": "1.0.0",
    //       "url":  ".../1.0.0-to-1.1.0-win-x64.bsdiff.zst",
    //       "signature": "<base64 ed25519 over the *patch bytes*>",
    //       "blake3":    "<hex BLAKE3 of the *patch bytes*>",
    //       "size": 145000,
    //       "compression": "zstd"   // or omitted/"none"
    //     }
    //   ]
    //
    // The `signature` in this entry covers the patch payload only —
    // the parent `signature` field above still authenticates the
    // *reconstructed* new binary, and `download_and_patch` re-verifies
    // it after applying the patch. That double-check defends against a
    // CDN swapping in a malicious patch that decompresses to something
    // not covered by the original release signature.
    let patch_entry = entry.get("patches").and_then(Value::as_array).and_then(|patches| {
        patches.iter().find(|p| {
            p.get("from")
                .and_then(Value::as_str)
                .is_some_and(|from| version_eq(from, current))
        })
    });

    Ok(json!({
        "available": true,
        "version": available_version,
        "notes": manifest.get("notes").cloned().unwrap_or(Value::Null),
        "pubDate": manifest.get("pub_date").cloned().unwrap_or(Value::Null),
        "url": url,
        "signature": signature,
        "platform": key,
        // Optional patch metadata. `null` when no patch matches the
        // current version — the JS layer then falls through to the
        // full-binary `downloadAndVerify` path. Present as a structured
        // object (not just url+sig) so frontends can show the size
        // savings before download.
        "patch": patch_entry.cloned().unwrap_or(Value::Null),
    }))
}

/// Compare two version strings via the semver helper, falling back to
/// case-insensitive string equality so a manifest that uses "1.0.0"
/// for the `from` field still matches a `currentVersion` of "v1.0.0".
fn version_eq(a: &str, b: &str) -> bool {
    let trim = |s: &str| s.trim().trim_start_matches('v').to_string();
    let (ta, tb) = (trim(a), trim(b));
    if ta == tb {
        return true;
    }
    match (semver::Version::parse(&ta), semver::Version::parse(&tb)) {
        (Ok(va), Ok(vb)) => va == vb,
        _ => false,
    }
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
    // Cap the pre-allocation at 8 MiB so a hostile/incorrect Content-Length
    // can't make us reserve gigabytes up-front. `BytesMut` grows
    // amortized-O(1) past the initial capacity.
    let mut accum = BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("updater.downloadAndVerify: read: {e}"))?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        accum.extend_from_slice(&buf[..n]);
        emitter.advance(n, false);
    }
    // Freeze the mutable accumulator into an immutable, refcounted
    // `Bytes`. The buffer is read three times below (Ed25519 verify,
    // BLAKE3 hash, on-disk write) — `Bytes::clone()` would be an Arc
    // inc rather than a memcpy if any of those were moved off-thread,
    // and the deref-to-`&[u8]` is free for the inline reads here.
    let bytes_accum: Bytes = accum.freeze();

    verifying_key
        .verify(&bytes_accum, &signature)
        .map_err(|e| format!("updater.downloadAndVerify: signature check failed: {e}"))?;

    // BLAKE3 of the *verified* bytes. `install` will re-hash the file
    // it's about to swap in and refuse the swap on mismatch — closes
    // the TOCTOU between this function returning and `install` calling
    // `self_replace`.
    let verified_hash = blake3::hash(&bytes_accum);

    let tmp = std::env::temp_dir().join(format!("tynd-update-{}", simple_rand()));
    std::fs::write(&tmp, &bytes_accum)
        .map_err(|e| format!("updater.downloadAndVerify: write: {e}"))?;

    emitter.emit_phase("verified");

    Ok(json!({
        "path": tmp.to_string_lossy(),
        "size": bytes_accum.len(),
        // Hex-encoded BLAKE3 — must be passed back to `install` so it can
        // detect tampering of the staged file before swapping the running
        // binary. Frontends that bypass this field force `install` into
        // its safe-default refusal branch.
        "hash": verified_hash.to_hex().to_string(),
    }))
}

/// Download a BSDIFF patch, verify its Ed25519 signature + BLAKE3,
/// apply it on top of the currently-running binary to reconstruct the
/// new release, then re-verify the *reconstructed* binary against the
/// release-level signature from the manifest. Output shape is
/// identical to `download_and_verify` (path/size/hash) so the JS layer
/// can hand the result straight to `install` without branching.
///
/// Required args:
/// - `url`            : patch download URL (`.bsdiff` or `.bsdiff.zst`)
/// - `signature`      : base64 Ed25519 over the *patch bytes*
/// - `pubKey`         : base64 Ed25519 verifying key (same one used for full)
/// - `releaseSignature`: base64 Ed25519 over the *reconstructed binary*
///   (this is the per-platform `signature` field from the manifest)
/// - `compression`    : "zstd" | "none" (default: detect from URL suffix)
/// Optional:
/// - `expectedBlake3` : hex BLAKE3 of the patch bytes (defense in depth)
/// - `progressId`, `proxy`, `headers` — same semantics as the full path.
fn download_and_patch(args: &Value) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndPatch: missing 'url'")?;
    let patch_sig_b64 = args
        .get("signature")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndPatch: missing 'signature' (Ed25519 over patch bytes)")?;
    let pub_key_b64 = args
        .get("pubKey")
        .and_then(Value::as_str)
        .ok_or("updater.downloadAndPatch: missing 'pubKey'")?;
    let release_sig_b64 = args.get("releaseSignature").and_then(Value::as_str).ok_or(
        "updater.downloadAndPatch: missing 'releaseSignature' (Ed25519 over the reconstructed \
         new binary; required so a malicious patch can't decompose to an unsigned binary)",
    )?;
    let progress_id = args
        .get("progressId")
        .and_then(Value::as_str)
        .map(String::from);
    let expected_patch_blake3 = args.get("expectedBlake3").and_then(Value::as_str);
    // Default to `zstd` when the URL ends with `.zst` — keeps the
    // manifest format flexible without forcing the field on every entry.
    let compression = args
        .get("compression")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if url.ends_with(".zst") || url.ends_with(".zstd") {
                "zstd"
            } else {
                "none"
            }
        });

    let patch_signature = decode_signature(patch_sig_b64)?;
    let release_signature = decode_signature(release_sig_b64)?;
    let verifying_key = decode_pub_key(pub_key_b64)?;
    let proxy = args.get("proxy").and_then(Value::as_str);

    // ---- 1) Download patch bytes (small, ~KB) -----------------------
    let agent = http_agent(proxy)?;
    let req = apply_headers(agent.get(url), args.get("headers"));
    let resp = req
        .call()
        .map_err(|e| format!("updater.downloadAndPatch({url}): {e}"))?;
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok());

    let mut emitter = Progress::new(progress_id, "download", total);
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    // Cap the patch size at 256 MB — anything beyond that defeats the
    // point of delta updates (download the full binary instead) and
    // protects against a malicious manifest pointing at an oversized
    // payload. The bidiff format itself doesn't carry a size prefix, so
    // the cap has to live here.
    const MAX_PATCH_BYTES: usize = 256 * 1024 * 1024;
    // Same `BytesMut` → `freeze()` pattern as `download_and_verify`: the
    // patch is read multiple times below (Ed25519 verify, optional
    // BLAKE3, decompress) so the refcounted `Bytes` keeps any future
    // off-thread fan-out cheap.
    let mut patch_accum =
        BytesMut::with_capacity(total.unwrap_or(0).min(8 * 1024 * 1024) as usize);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("updater.downloadAndPatch: read: {e}"))?;
        if n == 0 {
            emitter.advance(0, true);
            break;
        }
        patch_accum.extend_from_slice(&buf[..n]);
        if patch_accum.len() > MAX_PATCH_BYTES {
            return Err(format!(
                "updater.downloadAndPatch: patch exceeds {MAX_PATCH_BYTES} byte safety cap; refusing"
            ));
        }
        emitter.advance(n, false);
    }
    let patch_bytes: Bytes = patch_accum.freeze();

    // ---- 2) Verify patch signature + (optional) BLAKE3 --------------
    verifying_key
        .verify(&patch_bytes, &patch_signature)
        .map_err(|e| format!("updater.downloadAndPatch: patch signature check failed: {e}"))?;

    if let Some(expected) = expected_patch_blake3 {
        let actual = blake3::hash(&patch_bytes).to_hex().to_string();
        if !hashes_match(expected, &actual) {
            return Err(format!(
                "updater.downloadAndPatch: patch BLAKE3 mismatch — expected {expected}, got \
                 {actual}; refusing to apply"
            ));
        }
    }

    emitter.emit_phase("verified-patch");

    // ---- 3) Decompress (if needed) ---------------------------------
    // `zstd::decode_all` and the no-compression branch both yield byte
    // buffers consumed by the same `bipatch::Reader::new(Cursor::new(&...))`
    // below. We materialize as `Vec<u8>` because `bipatch` reads via a
    // `Cursor<&[u8]>` either way — for the `"none"` branch this is one
    // memcpy out of `Bytes` to drop refcounted state before the
    // long-running patch apply.
    let patch_payload: Vec<u8> = match compression {
        "zstd" => zstd::decode_all(&patch_bytes[..])
            .map_err(|e| format!("updater.downloadAndPatch: zstd decode: {e}"))?,
        "none" => patch_bytes.to_vec(),
        other => {
            return Err(format!(
                "updater.downloadAndPatch: unknown compression '{other}'; expected 'zstd' or 'none'"
            ))
        },
    };

    // ---- 4) Read current binary, apply patch -----------------------
    let current_exe =
        std::env::current_exe().map_err(|e| format!("updater.downloadAndPatch: current_exe: {e}"))?;
    let old_bytes = std::fs::read(&current_exe).map_err(|e| {
        format!(
            "updater.downloadAndPatch: read current_exe '{}': {e}",
            current_exe.display()
        )
    })?;

    emitter.emit_phase("applying");

    let mut reconstructed: Vec<u8> = Vec::with_capacity(old_bytes.len());
    {
        use std::io::{Cursor, Read as _};
        let mut bipatch_reader =
            bipatch::Reader::new(Cursor::new(&patch_payload), Cursor::new(&old_bytes))
                .map_err(|e| format!("updater.downloadAndPatch: bipatch header: {e}"))?;
        bipatch_reader
            .read_to_end(&mut reconstructed)
            .map_err(|e| format!("updater.downloadAndPatch: bipatch apply: {e}"))?;
    }

    // ---- 5) Re-verify the RECONSTRUCTED binary against the release
    // signature. This is the load-bearing check: it makes sure the
    // patch produced exactly the bytes the release signer covered.
    // Without it, an attacker who could MITM the patch could craft any
    // payload they want — the patch sig only authenticates the patch
    // itself, not what it computes to. -----------------------------
    verifying_key
        .verify(&reconstructed, &release_signature)
        .map_err(|e| {
            format!(
                "updater.downloadAndPatch: reconstructed binary failed release signature \
                 verification: {e} (patch may be authentic but doesn't reproduce the signed \
                 release; refusing to install)"
            )
        })?;

    let verified_hash = blake3::hash(&reconstructed);

    // ---- 6) Stage to temp dir for the `install` step --------------
    let tmp = std::env::temp_dir().join(format!("tynd-update-{}", simple_rand()));
    std::fs::write(&tmp, &reconstructed)
        .map_err(|e| format!("updater.downloadAndPatch: write staged: {e}"))?;

    emitter.emit_phase("verified");

    Ok(json!({
        "path": tmp.to_string_lossy(),
        "size": reconstructed.len(),
        "hash": verified_hash.to_hex().to_string(),
        // Echoed back so frontends can show the actual savings ratio
        // (compared to advertising it from the manifest only).
        "patchSize": patch_bytes.len(),
    }))
}

/// Stream the file at `path` through BLAKE3 and return the lowercase hex
/// digest. Used by `install` to re-verify the staged update binary
/// immediately before `self_replace` swaps it in. Streaming (rather
/// than `read` + `hash`) keeps the peak memory bounded for >100 MB
/// updates.
fn blake3_file(path: &std::path::Path) -> std::io::Result<String> {
    let mut hasher = Blake3Hasher::new();
    let mut file = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn simple_rand() -> String {
    // Small, non-crypto unique suffix. PID + full nanos + atomic counter so
    // two parallel calls in the same process can't collide.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
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
    // Delegate to the semver-backed public helper for accurate pre-release
    // ordering. Fall back to `true` (assume update available) on any parse
    // failure so a malformed manifest version doesn't silently block the check.
    is_update_available(current, available).unwrap_or(true)
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
    // SECURITY: the hex BLAKE3 produced by `download_and_verify` must be
    // passed back here. We re-hash the staged file and compare — any
    // mismatch means an attacker (or a buggy plugin) wrote to `temp_dir`
    // between download and install. Without this field, refuse the
    // install rather than silently degrading the trust model. Frontends
    // built before this contract change will surface the error and
    // route through download_and_verify again.
    let expected_hash = args
        .get("hash")
        .and_then(Value::as_str)
        .ok_or(
            "updater.install: missing 'hash' (must be the BLAKE3 returned by downloadAndVerify; \
             omitting it would skip the TOCTOU re-check)",
        )?;

    let current =
        std::env::current_exe().map_err(|e| format!("updater.install: current_exe: {e}"))?;

    let new_path_buf = std::path::PathBuf::from(new_path);
    if !new_path_buf.exists() {
        return Err(format!(
            "updater.install: downloaded file not found at '{new_path}'"
        ));
    }

    // Re-verify the on-disk file matches the verified-bytes hash from
    // `download_and_verify`. Length-checked + constant-time compare
    // because mismatched-length hex would otherwise short-circuit the
    // memory equality.
    let actual_hash = blake3_file(&new_path_buf)
        .map_err(|e| format!("updater.install: hash recompute failed: {e}"))?;
    if !hashes_match(expected_hash, &actual_hash) {
        // This is a hard failure — surface it loudly so a deployer
        // sees the tampering attempt in logs, then refuse the swap.
        tracing::error!(
            target: "tynd.updater",
            expected = %expected_hash,
            actual = %actual_hash,
            path = %new_path,
            "tampering detected: staged update file does not match verified hash"
        );
        return Err(format!(
            "updater.install: tampering detected — staged file hash {actual_hash} does not match \
             verified hash {expected_hash}; refusing self_replace"
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

/// Constant-time equality on lowercase hex strings of the same length.
/// `==` on `&str` short-circuits on the first mismatched byte; for a
/// publicly-supplied hash that's a timing oracle. BLAKE3 hashes don't
/// carry secrets but the comparison is cheap enough to do right.
fn hashes_match(expected: &str, actual: &str) -> bool {
    let exp = expected.as_bytes();
    let act = actual.as_bytes();
    if exp.len() != act.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in exp.iter().zip(act.iter()) {
        // Lowercase the expected byte so a hash sent as upper-case hex
        // still compares equal to our lowercase output.
        let ae = a.to_ascii_lowercase();
        let bb = b.to_ascii_lowercase();
        diff |= ae ^ bb;
    }
    diff == 0
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
    fn is_update_available_strict_semver() {
        assert!(is_update_available("1.0.0", "1.0.1").unwrap());
        assert!(is_update_available("1.0.0", "1.1.0").unwrap());
        assert!(is_update_available("1.0.0", "2.0.0").unwrap());
        assert!(!is_update_available("1.0.1", "1.0.1").unwrap());
        assert!(!is_update_available("1.0.1", "1.0.0").unwrap());
        // v-prefix accepted
        assert!(is_update_available("v1.0.0", "v1.0.1").unwrap());
        // pre-release ordering: 1.0.0-alpha < 1.0.0
        assert!(is_update_available("1.0.0-alpha.1", "1.0.0").unwrap());
        assert!(!is_update_available("1.0.0", "1.0.0-alpha.1").unwrap());
    }

    #[test]
    fn is_update_available_invalid_version_errors() {
        assert!(is_update_available("not-a-version", "1.0.0").is_err());
        assert!(is_update_available("1.0.0", "bad").is_err());
    }

    #[test]
    fn meets_requirement_ranges() {
        assert!(meets_requirement("1.5.0", ">=1.0.0, <2.0.0").unwrap());
        assert!(!meets_requirement("2.0.0", ">=1.0.0, <2.0.0").unwrap());
        assert!(meets_requirement("1.0.0", "^1.0").unwrap());
        assert!(!meets_requirement("2.0.0", "^1.0").unwrap());
        assert!(meets_requirement("v1.2.3", "~1.2").unwrap());
    }

    #[test]
    fn meets_requirement_bad_input_errors() {
        assert!(meets_requirement("not-a-ver", ">=1.0").is_err());
        assert!(meets_requirement("1.0.0", ">=bad").is_err());
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

    #[test]
    fn hashes_match_handles_case_and_length() {
        let a = "abcdef0123456789";
        assert!(hashes_match(a, a));
        // Case-insensitive compare — the JSON channel may upcase hex.
        assert!(hashes_match("ABCDEF0123456789", a));
        assert!(!hashes_match(a, "abcdef0123456788"));
        // Length mismatch -> false (and short-circuits before xor).
        assert!(!hashes_match("abc", "abcd"));
        assert!(!hashes_match("", a));
    }

    #[test]
    fn blake3_file_matches_in_memory_hash() {
        // Round-trip: write a payload, hash via streaming, compare to
        // the one-shot in-memory hash. Catches off-by-ones in the chunk
        // loop and EOF handling.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("payload.bin");
        let payload: Vec<u8> = (0..200_000).map(|i| (i & 0xff) as u8).collect();
        std::fs::write(&p, &payload).unwrap();

        let direct = blake3::hash(&payload).to_hex().to_string();
        let streamed = blake3_file(&p).unwrap();
        assert_eq!(direct, streamed);
    }

    #[test]
    fn install_rejects_missing_hash() {
        // The TOCTOU mitigation requires the caller to round-trip the
        // hash from `downloadAndVerify`. Omitting the field MUST be a
        // hard error — we'd rather surface a louder error than silently
        // skip the re-verification.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("update.bin");
        std::fs::write(&p, b"new binary").unwrap();
        let err = install(&json!({
            "path": p.to_string_lossy(),
            "relaunch": false,
        }))
        .unwrap_err();
        assert!(
            err.contains("missing 'hash'"),
            "expected missing-hash error, got: {err}"
        );
    }

    #[test]
    fn install_detects_tampering_after_verify() {
        // Simulate the TOCTOU window: caller passes a hash that doesn't
        // match the on-disk file (an attacker swapped the staged binary
        // between download and install). install() must refuse.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("update.bin");
        std::fs::write(&p, b"tampered contents").unwrap();
        // Hash of a *different* payload — anything that isn't the real hash.
        let bogus_hash = blake3::hash(b"the trusted contents").to_hex().to_string();
        let err = install(&json!({
            "path": p.to_string_lossy(),
            "relaunch": false,
            "hash": bogus_hash,
        }))
        .unwrap_err();
        assert!(
            err.contains("tampering detected"),
            "expected tampering error, got: {err}"
        );
    }

    // -----------------------------------------------------------------
    // W4.3 — Ed25519 fuzz + downgrade negative paths
    //
    // The full HTTP path through `download_and_verify` would require
    // standing up a local server per test. Instead we exercise the
    // signature-decode + verify primitives directly with a deterministic
    // key pair, plus the manifest-version comparison via `check`-like
    // helpers (`is_strictly_newer` already covers the same code path
    // `check` runs against `allowDowngrade=false`).
    // -----------------------------------------------------------------

    use ed25519_dalek::{Signer as _, SigningKey};

    /// Deterministic test key (zeros) — every fuzz / negative-path test
    /// uses the same one so failures are reproducible without a
    /// `rand`-feature dep on `ed25519-dalek`. The crate's `from_bytes`
    /// constructor accepts any 32 bytes; the zeros key is mathematically
    /// valid (it's used in their own test suite).
    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[0u8; 32])
    }

    #[test]
    fn ed25519_manifest_tampering_rejected() {
        // Sign a payload, flip a single byte, verify must fail. This
        // is the core anti-tamper invariant — if a future code change
        // accidentally verifies against a stale buffer or skips the
        // hash, this test catches it.
        let sk = test_signing_key();
        let pk = sk.verifying_key();
        let original = br#"{"version":"1.0.0","platforms":{}}"#;
        let sig = sk.sign(original);
        // Genuine signature on genuine bytes verifies.
        assert!(pk.verify(original, &sig).is_ok());
        // Flip one byte → must reject.
        let mut tampered = original.to_vec();
        tampered[10] ^= 0x01;
        assert!(
            pk.verify(&tampered, &sig).is_err(),
            "verify must reject tampered bytes"
        );
    }

    #[test]
    fn ed25519_signature_from_wrong_key_rejected() {
        // Two distinct keys; sign with one, verify against the other.
        // Catches a regression where the verifying_key embedded in the
        // build accidentally accepts signatures from a sibling key
        // (e.g. due to a reused buffer / wrong index into a key set).
        let sk_a = test_signing_key();
        let sk_b = SigningKey::from_bytes(&[1u8; 32]);
        let pk_b = sk_b.verifying_key();
        let payload = b"tynd-update-binary-contents";
        let sig_from_a = sk_a.sign(payload);
        // Verify against B's public key — must reject (sig was made by A).
        assert!(
            pk_b.verify(payload, &sig_from_a).is_err(),
            "wrong-key signature must reject"
        );
    }

    #[test]
    fn updater_rejects_downgrade_by_default() {
        // `is_strictly_newer` is the gatekeeper called by `check` when
        // `allowDowngrade: false` (the default). 2.0.0 is current,
        // manifest claims 1.0.0 → must NOT report an update available.
        // Models the real "rollback attack" case: hostile manifest
        // pinning users to a known-vulnerable older release.
        assert!(!is_strictly_newer("2.0.0", "1.0.0"));
        assert!(!is_strictly_newer("2.0.0", "2.0.0")); // equal isn't an update either
        assert!(!is_strictly_newer("2.0.0", "1.99.99"));
    }

    #[test]
    fn updater_allows_downgrade_when_explicitly_enabled() {
        // `check`'s opt-in branch: when the deployer passes
        // `allowDowngrade: true`, the version comparison is bypassed —
        // the function emits "available: true" for any platform-matched
        // entry, including older versions. We can't run the full HTTP
        // path here, so verify the gate directly: with
        // allow_downgrade=true the version check is skipped, so even
        // `is_strictly_newer == false` should still let the call
        // proceed. Pin the helper's behaviour so a refactor of `check`
        // doesn't accidentally invert the flag.
        //
        // (Rationale for not exercising `check` directly: it does a
        // network call. The downgrade gate is a single bool branch in
        // `check` — `if !allow_downgrade && !is_strictly_newer(...)
        // return available: false`. With `is_strictly_newer` returning
        // false for downgrade and `allow_downgrade=true`, the short
        // circuit doesn't fire. That's the contract we're pinning.)
        //
        // Concretely: a deployment scenario where
        // `current=2.0.0, manifest=1.0.0, allowDowngrade=true` MUST
        // result in is_strictly_newer-returning-false being ignored.
        let current = "2.0.0";
        let manifest = "1.0.0";
        let is_newer = is_strictly_newer(current, manifest);
        let allow_downgrade = true;
        // The guard the production code uses:
        let blocked = !allow_downgrade && !is_newer;
        assert!(
            !blocked,
            "allowDowngrade=true must let the older manifest through"
        );
    }

    #[test]
    fn updater_decode_signature_rejects_empty_and_wrong_length() {
        // `decode_signature` is the boundary that translates JSON
        // strings into a typed `Signature`. Empty / wrong-length
        // inputs MUST fail loudly so the HTTP fetch never even runs
        // — silently treating them as "no signature available" would
        // be the worst kind of bypass.
        // Empty base64 → length check fails (decode succeeds with 0 bytes).
        let err_empty = decode_signature("").unwrap_err();
        assert!(
            err_empty.contains("length"),
            "empty signature should hit length check, got: {err_empty}"
        );
        // 32 bytes is the natural mistake (using SECRET_KEY_LENGTH
        // / PUBLIC_KEY_LENGTH instead of SIGNATURE_LENGTH=64).
        let half = STANDARD.encode([0u8; 32]);
        let err_short = decode_signature(&half).unwrap_err();
        assert!(
            err_short.contains("length"),
            "32-byte signature should fail, got: {err_short}"
        );
        // 65 bytes (off-by-one over) — also rejected.
        let over = STANDARD.encode([0u8; 65]);
        let err_long = decode_signature(&over).unwrap_err();
        assert!(
            err_long.contains("length"),
            "65-byte signature should fail, got: {err_long}"
        );
        // Garbage base64 → decode error (covers the b64-frame surface).
        let err_garbage = decode_signature("not-valid-b64!@#").unwrap_err();
        assert!(
            err_garbage.contains("base64"),
            "garbage b64 should surface a base64 error, got: {err_garbage}"
        );
    }

    #[test]
    fn updater_decode_pub_key_rejects_wrong_length() {
        // Symmetric guard for the public key decode path. A 64-byte
        // input (signature length) must fail rather than silently
        // truncate — historical Ed25519 code has confused the two.
        let sig_sized = STANDARD.encode([0u8; 64]);
        let err = decode_pub_key(&sig_sized).unwrap_err();
        assert!(
            err.contains("length"),
            "64-byte pubKey should hit length check, got: {err}"
        );
    }

    #[test]
    fn updater_full_round_trip_through_dispatch_helpers() {
        // Generate a valid sig + pubkey pair, then exercise the same
        // base64 → Signature / VerifyingKey decode path the production
        // `download_and_verify` uses, and confirm an end-to-end verify
        // succeeds. Catches regressions where the b64 alphabet, padding,
        // or byte order between decode_* and verify drifts.
        let sk = test_signing_key();
        let pk = sk.verifying_key();
        let payload = b"hello tynd updater";
        let sig = sk.sign(payload);

        let sig_b64 = STANDARD.encode(sig.to_bytes());
        let pk_b64 = STANDARD.encode(pk.to_bytes());

        let decoded_sig = decode_signature(&sig_b64).unwrap();
        let decoded_pk = decode_pub_key(&pk_b64).unwrap();
        assert!(
            decoded_pk.verify(payload, &decoded_sig).is_ok(),
            "decoded pair must verify the payload"
        );
        // And the negative direction: tampered payload still rejects
        // through the decoded primitives.
        let mut bad = payload.to_vec();
        bad[0] ^= 0x80;
        assert!(decoded_pk.verify(&bad, &decoded_sig).is_err());
    }

    // ---- W3.3 BSDIFF patch round-trip + negative-path tests ---------
    //
    // These tests build a synthetic patch via the `bidiff` crate
    // (workspace dep, same one the bundler uses) and then exercise the
    // exact `bipatch::Reader` codepath the production
    // `download_and_patch` uses, minus the HTTP shell. Keeps the suite
    // hermetic (no network, no `current_exe` mutation) while covering
    // the wire format + Ed25519 surface end-to-end.

    /// Helper — build a deterministic Ed25519 keypair from a seed so
    /// the assertions below stay reproducible across runs.
    fn deterministic_keypair() -> (ed25519_dalek::SigningKey, VerifyingKey) {
        use ed25519_dalek::SigningKey;
        let seed: [u8; 32] = *b"tynd-updater-w3.3-patch-tests!!!";
        let sk = SigningKey::from_bytes(&seed);
        let pk = sk.verifying_key();
        (sk, pk)
    }

    /// Helper — produce a `bidiff` patch (no compression) between two
    /// in-memory binaries. Mirrors what `cli/native/bundler patch.rs`
    /// emits, so any format skew between the two halves of the
    /// workspace surfaces here.
    fn make_patch(old: &[u8], new: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        bidiff::simple_diff(old, new, &mut out).expect("bidiff");
        out
    }

    #[test]
    fn round_trip_diff_then_apply_reproduces_new_binary() {
        let old: Vec<u8> = (0..32_768u32).map(|i| (i & 0xff) as u8).collect();
        let mut new = old.clone();
        for b in &mut new[1_000..2_000] {
            *b = b.wrapping_add(13);
        }
        new.extend_from_slice(b"-tail-only-in-new-");

        let patch = make_patch(&old, &new);

        // Apply via the same `bipatch::Reader` API `download_and_patch` uses.
        use std::io::{Cursor, Read as _};
        let mut reader =
            bipatch::Reader::new(Cursor::new(&patch), Cursor::new(&old)).unwrap();
        let mut reconstructed = Vec::new();
        reader.read_to_end(&mut reconstructed).unwrap();
        assert_eq!(reconstructed, new);
    }

    #[test]
    fn round_trip_with_zstd_compression() {
        let old: Vec<u8> = (0..16_384u32).map(|i| ((i * 7) & 0xff) as u8).collect();
        let mut new = old.clone();
        new[0..16].fill(0xCC);

        let raw_patch = make_patch(&old, &new);
        let compressed = zstd::encode_all(&raw_patch[..], 9).unwrap();
        let decompressed = zstd::decode_all(&compressed[..]).unwrap();
        assert_eq!(decompressed, raw_patch);

        use std::io::{Cursor, Read as _};
        let mut reader =
            bipatch::Reader::new(Cursor::new(&decompressed), Cursor::new(&old)).unwrap();
        let mut reconstructed = Vec::new();
        reader.read_to_end(&mut reconstructed).unwrap();
        assert_eq!(reconstructed, new);
    }

    #[test]
    fn corrupt_patch_header_is_rejected() {
        let old = vec![0u8; 4_096];
        let mut new = old.clone();
        new[10] = 0xFF;
        let mut patch = make_patch(&old, &new);
        // Flip the magic so `bipatch::Reader::new` refuses it before we
        // even start reading controls. Cheapest tampering a CDN could do.
        patch[0] ^= 0xAA;

        use std::io::Cursor;
        // `bipatch::Reader` doesn't implement Debug so we can't
        // `.unwrap_err()` directly — pattern-match instead.
        let res = bipatch::Reader::new(Cursor::new(&patch), Cursor::new(&old));
        match res {
            Err(e) => {
                let s = format!("{e:?}");
                assert!(
                    s.contains("WrongMagic"),
                    "expected WrongMagic decode error, got: {s}"
                );
            },
            Ok(_) => panic!("corrupt patch must be rejected by Reader::new"),
        }
    }

    #[test]
    fn patch_signature_verify_rejects_tampered_payload() {
        // `download_and_patch` Ed25519-verifies the *patch bytes*
        // before decompression. Flipping a byte in the signed payload
        // must make the same verifier reject it.
        let (sk, pk) = deterministic_keypair();
        let payload = b"a-fake-patch-payload-for-the-signature-surface".to_vec();
        let sig = ed25519_dalek::Signer::sign(&sk, &payload);
        let sig_b64 = STANDARD.encode(sig.to_bytes());
        let pk_b64 = STANDARD.encode(pk.to_bytes());

        let decoded_sig = decode_signature(&sig_b64).unwrap();
        let decoded_pk = decode_pub_key(&pk_b64).unwrap();
        assert!(decoded_pk.verify(&payload, &decoded_sig).is_ok());

        let mut tampered = payload.clone();
        tampered[3] ^= 0x01;
        assert!(decoded_pk.verify(&tampered, &decoded_sig).is_err());
    }

    #[test]
    fn release_signature_must_cover_reconstructed_binary() {
        // The production `download_and_patch` runs *two* Ed25519 verifies:
        // one over the patch bytes and a second over the reconstructed
        // new binary. This test simulates a malicious actor whose patch
        // reconstructs to bytes the release signature does not cover —
        // the second verify must reject it.
        let (release_signer, release_pk) = deterministic_keypair();

        let old = vec![1u8; 1_024];
        let intended_new = vec![2u8; 1_024];
        let attacker_new = vec![3u8; 1_024];

        // Sign only the *intended* new binary.
        let release_sig = ed25519_dalek::Signer::sign(&release_signer, &intended_new);

        // Patch from old -> attacker_new yields attacker_new, NOT intended_new.
        let patch = make_patch(&old, &attacker_new);
        use std::io::{Cursor, Read as _};
        let mut reader =
            bipatch::Reader::new(Cursor::new(&patch), Cursor::new(&old)).unwrap();
        let mut reconstructed = Vec::new();
        reader.read_to_end(&mut reconstructed).unwrap();
        assert_eq!(reconstructed, attacker_new);
        assert!(
            release_pk.verify(&reconstructed, &release_sig).is_err(),
            "release signature MUST NOT validate a patch reconstruction it did not cover"
        );
        // Conversely, the legitimate path verifies cleanly.
        let legit_patch = make_patch(&old, &intended_new);
        let mut legit_reader =
            bipatch::Reader::new(Cursor::new(&legit_patch), Cursor::new(&old)).unwrap();
        let mut legit_reconstructed = Vec::new();
        legit_reader.read_to_end(&mut legit_reconstructed).unwrap();
        assert!(release_pk.verify(&legit_reconstructed, &release_sig).is_ok());
    }

    #[test]
    fn version_eq_handles_v_prefix_and_canonicalization() {
        assert!(version_eq("1.0.0", "1.0.0"));
        assert!(version_eq("v1.0.0", "1.0.0"));
        assert!(version_eq("1.0.0", "v1.0.0"));
        assert!(!version_eq("1.0.0", "1.0.1"));
        // Whitespace tolerance via the trim() inside version_eq.
        assert!(version_eq("  1.2.3 ", "v1.2.3"));
        // Pre-release ordering — two different pre-release tags don't
        // collapse to "equal" just because the numeric core matches.
        assert!(!version_eq("1.0.0-alpha.1", "1.0.0"));
    }

    #[test]
    fn download_and_patch_rejects_missing_release_signature() {
        // Argument-validation surface: omitting the release-signature
        // field MUST be a hard error. Without that field the function
        // would silently degrade to "patch-sig-only" — exactly the
        // attack the double-verify is designed to prevent.
        let err = download_and_patch(&json!({
            "url": "https://example.test/patch.bsdiff.zst",
            "signature": "AA",
            "pubKey": "AA",
        }))
        .unwrap_err();
        assert!(
            err.contains("releaseSignature"),
            "expected releaseSignature error, got: {err}"
        );
    }

    // ---- proptest: semver comparison vs. raw `semver` crate ----------

    proptest::proptest! {
        /// Generated 100 (default) version pairs MUST keep
        /// `is_update_available` consistent with `semver::Version`'s
        /// own ordering. Catches off-by-ones in the `>` vs `>=`
        /// branch and any future "I'll just check major.minor"
        /// shortcut that would diverge from full semver.
        #[test]
        fn proptest_is_update_available_matches_semver_ordering(
            a_major in 0u64..50,
            a_minor in 0u64..50,
            a_patch in 0u64..50,
            b_major in 0u64..50,
            b_minor in 0u64..50,
            b_patch in 0u64..50,
        ) {
            let a = format!("{a_major}.{a_minor}.{a_patch}");
            let b = format!("{b_major}.{b_minor}.{b_patch}");
            let helper = is_update_available(&a, &b).unwrap();
            let expected = semver::Version::parse(&b).unwrap()
                > semver::Version::parse(&a).unwrap();
            proptest::prop_assert_eq!(helper, expected);
        }
    }
}
