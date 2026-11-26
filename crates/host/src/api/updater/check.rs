use std::{io::Read, time::Duration};

use serde_json::{json, Value};
use tynd_update_core::{
    render_endpoint, rollout_allows, rollout_bucket, try_endpoints, FetchFailure, TemplateVars,
    UpdaterConfig, DEFAULT_CHANNEL,
};

use super::{
    authorize_manifest_downloads, decode_signature, explicit_channel,
    get_with_redirects_classified, is_strictly_newer, platform_key, platform_parts, state,
    version_eq, KeySet,
};

const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn check(args: &Value) -> Result<Value, String> {
    check_with_timeout(args, super::DEFAULT_HTTP_TIMEOUT)
}

/// Periodic checks must not make host shutdown wait for the full manual
/// download timeout. A failed background check is retried at the next
/// interval, so a short global bound is preferable to an orphaned worker.
pub(super) fn check_periodic(args: &Value) -> Result<Value, String> {
    check_with_timeout(args, super::PERIODIC_HTTP_TIMEOUT)
}

/// What a check runs against. Everything here comes from the host (trusted
/// updater config + the app's own version), never from the IPC arguments.
struct CheckInputs {
    templates: Vec<String>,
    current: String,
    default_channel: Option<String>,
}

/// The app's own version, as declared by the backend through `app.setInfo`.
fn trusted_version(version: Option<String>) -> Result<String, String> {
    version.filter(|v| !v.trim().is_empty()).ok_or_else(|| {
        "updater.check: the app version is unknown - the backend must call `app.setInfo({ version })` at startup (the version is never taken from the caller)"
            .to_string()
    })
}

fn check_inputs(cfg: &UpdaterConfig, version: Option<String>) -> Result<CheckInputs, String> {
    let templates = cfg.resolved_endpoints();
    if templates.is_empty() {
        return Err("updater.check: the trusted updater config has no endpoint".into());
    }
    Ok(CheckInputs {
        templates,
        current: trusted_version(version)?,
        default_channel: cfg.channel.clone(),
    })
}

/// Signature URL for each rendered endpoint: every mirror publishes
/// `<endpoint>.sig` next to its manifest.
fn signature_urls(endpoints: &[String]) -> Vec<String> {
    endpoints
        .iter()
        .map(|endpoint| format!("{endpoint}.sig"))
        .collect()
}

/// Templates rendered for this install, in fallback order.
fn render_endpoints(templates: &[String], channel: &str, current: &str) -> Vec<String> {
    let (target, arch) = platform_parts();
    let vars = TemplateVars {
        channel,
        target,
        arch,
        current_version: current,
    };
    templates
        .iter()
        .map(|template| render_endpoint(template, &vars))
        .collect()
}

/// `allowDowngrade: true` and `requireSignature: false` are accepted on the
/// wire only to be refused: neither may weaken a check.
fn reject_unsafe_check_options(args: &Value) -> Result<(), String> {
    if args
        .get("allowDowngrade")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err("updater.check: allowDowngrade is not permitted".into());
    }
    let require_signature = args
        .get("requireSignature")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !require_signature {
        return Err("updater.check: unsigned manifests are not permitted".into());
    }
    Ok(())
}

/// Read `(url, signature, format)` from the manifest's platform entry and
/// reject formats this platform cannot install.
fn platform_artifact<'a>(
    entry: &'a Value,
    key: &str,
) -> Result<(&'a str, &'a str, &'a str), String> {
    let url = entry
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("updater.check: platform '{key}' missing 'url'"))?;
    let signature = entry
        .get("signature")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("updater.check: platform '{key}' missing 'signature'"))?;
    let format = entry
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("binary");
    if !matches!(format, "binary" | "app-archive") {
        return Err(format!(
            "updater.check: platform '{key}' has unsupported format '{format}'"
        ));
    }
    if cfg!(target_os = "macos") && format != "app-archive" {
        return Err(
            "updater.check: macOS updates must use format 'app-archive' to preserve the .app signature"
                .into(),
        );
    }
    Ok((url, signature, format))
}

fn check_with_timeout(args: &Value, timeout: Duration) -> Result<Value, String> {
    let inputs = check_inputs(
        &super::trusted_config()?,
        crate::api::app::configured_version(),
    )?;
    let current = inputs.current.as_str();
    reject_unsafe_check_options(args)?;

    // The channel is the one the install follows (persisted `setChannel`, else
    // the trusted config default); a caller-supplied `channel` is ignored.
    let explicit = explicit_channel(inputs.default_channel.as_deref());
    let channel = explicit.as_deref().unwrap_or(DEFAULT_CHANNEL);
    let endpoints = render_endpoints(&inputs.templates, channel, current);
    let keys = KeySet::from_trusted("updater.check")?;
    let signature_urls = signature_urls(&endpoints);

    let agent = super::http_agent_with_timeout(timeout)?;
    let (body, _served_by) = try_endpoints(&endpoints, |index, endpoint| {
        fetch_verified_manifest(&agent, &keys, endpoint, &signature_urls[index])
    })?;

    let manifest: Value =
        serde_json::from_slice(&body).map_err(|e| format!("updater.check: manifest parse: {e}"))?;

    let available_version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("updater.check: manifest missing 'version'")?
        .to_string();

    if !is_strictly_newer(current, &available_version) {
        return Ok(json!({ "available": false }));
    }

    check_manifest_channel(&manifest, explicit.as_deref())?;
    let rollout = manifest.get("rollout");
    if !rollout_includes_install(rollout, &state::install_id(), &available_version)? {
        return Ok(json!({ "available": false, "rolloutHeld": true }));
    }

    let key = platform_key();
    let entry = manifest
        .get("platforms")
        .and_then(|p| p.get(&key))
        .ok_or_else(|| format!("updater.check: no platform entry for '{key}'"))?;

    let (url, signature, format) = platform_artifact(entry, &key)?;

    let patch_entry = matching_patch(entry, current, format);

    authorize_manifest_downloads(url, signature, format, patch_entry)?;

    Ok(json!({
        "available": true,
        "version": available_version,
        "notes": manifest.get("notes").cloned().unwrap_or(Value::Null),
        "pubDate": manifest.get("pub_date").cloned().unwrap_or(Value::Null),
        "url": url,
        "signature": signature,
        "platform": key,
        "format": format,
        "channel": manifest.get("channel").cloned().unwrap_or(Value::Null),
        "rollout": rollout.cloned().unwrap_or(Value::Null),
        // Optional patch metadata. `null` when no patch matches the
        // current version — the JS layer then falls through to the
        // full-binary `downloadAndVerify` path. Present as a structured
        // object (not just url+sig) so frontends can show the size
        // savings before download.
        "patch": patch_entry.cloned().unwrap_or(Value::Null),
    }))
}

fn matching_patch<'a>(entry: &'a Value, current: &str, format: &str) -> Option<&'a Value> {
    if format != "binary" {
        return None;
    }
    // Patch signature covers patch payload. Parent release signature still
    // authenticates reconstructed binary after patch application.
    entry
        .get("patches")
        .and_then(Value::as_array)
        .and_then(|patches| {
            patches.iter().find(|patch| {
                patch
                    .get("from")
                    .and_then(Value::as_str)
                    .is_some_and(|from| version_eq(from, current))
            })
        })
}

/// A manifest that names a channel must match the one this install follows
/// (when the app set one explicitly); this stops a mirror from replaying the
/// manifest of another channel. A manifest without a `channel` (`tynd updater publish`
/// always writes one, but hand-written manifests may omit it) is the default
/// channel's manifest, so it is accepted only by installs following
/// [`DEFAULT_CHANNEL`].
fn check_manifest_channel(manifest: &Value, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = manifest
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_CHANNEL);
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "updater.check: manifest is for channel '{actual}' but this install follows \
             '{expected}'"
        ))
    }
}

/// Staged rollout: `true` when this install's stable bucket falls inside the
/// manifest's `rollout` percentage (absent = everyone).
fn rollout_includes_install(
    rollout: Option<&Value>,
    install_id: &str,
    version: &str,
) -> Result<bool, String> {
    let percent = match rollout {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_f64()
                .ok_or("updater.check: manifest 'rollout' must be a number 0..100")?,
        ),
    };
    rollout_allows(percent, rollout_bucket(install_id, version))
        .map_err(|e| format!("updater.check: {e}"))
}

fn read_capped(
    resp: &mut ureq::http::Response<ureq::Body>,
    cap: usize,
) -> Result<Vec<u8>, FetchFailure> {
    let mut body = Vec::new();
    resp.body_mut()
        .as_reader()
        .take((cap + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|e| FetchFailure::Retryable(format!("updater.check: read manifest: {e}")))?;
    if body.len() > cap {
        return Err(FetchFailure::Fatal(format!(
            "updater.check: manifest exceeds {cap}-byte cap"
        )));
    }
    Ok(body)
}

fn require_success(
    resp: &ureq::http::Response<ureq::Body>,
    what: &str,
    url: &str,
) -> Result<(), FetchFailure> {
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(FetchFailure::Fatal(format!(
            "updater.check: {what} {} returned HTTP {}",
            super::redact_url(url),
            status.as_u16()
        )))
    }
}

/// Download one endpoint's manifest and its detached signature, and check the
/// signature against every trusted key. Transport failures are retryable
/// (the caller moves on to the next mirror); a bad signature is fatal.
fn fetch_verified_manifest(
    agent: &ureq::Agent,
    keys: &KeySet,
    endpoint: &str,
    sig_url: &str,
) -> Result<Vec<u8>, FetchFailure> {
    let mut resp = get_with_redirects_classified(agent, endpoint, "updater.check")?;
    require_success(&resp, "manifest", endpoint)?;
    let body = read_capped(&mut resp, MAX_MANIFEST_BYTES)?;

    let mut sig_resp = get_with_redirects_classified(agent, sig_url, "updater.check signature")?;
    require_success(&sig_resp, "manifest signature", sig_url)?;
    let mut sig_body = String::new();
    sig_resp
        .body_mut()
        .as_reader()
        .take(4096)
        .read_to_string(&mut sig_body)
        .map_err(|e| {
            FetchFailure::Retryable(format!("updater.check: read manifest signature: {e}"))
        })?;
    let signature = decode_signature(sig_body.trim()).map_err(FetchFailure::Fatal)?;
    keys.verify(&body, &signature).map_err(|e| {
        FetchFailure::Fatal(format!(
            "updater.check: manifest signature check failed: {e}"
        ))
    })?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: Value) -> UpdaterConfig {
        serde_json::from_value(json).unwrap()
    }

    fn trusted() -> UpdaterConfig {
        config(json!({
            "endpoints": ["https://cdn.example/{{channel}}/m.json", "https://mirror.example/m.json"],
            "publicKeys": ["K"],
            "channel": "beta",
        }))
    }

    #[test]
    fn inputs_come_from_the_trusted_config_and_app_version() {
        let inputs = check_inputs(&trusted(), Some("1.4.0".into())).unwrap();
        assert_eq!(
            inputs.templates,
            [
                "https://cdn.example/{{channel}}/m.json",
                "https://mirror.example/m.json"
            ]
        );
        assert_eq!(inputs.current, "1.4.0");
        assert_eq!(inputs.default_channel.as_deref(), Some("beta"));
    }

    #[test]
    fn unknown_app_version_refuses_instead_of_defaulting() {
        assert!(check_inputs(&trusted(), None).is_err());
        assert!(check_inputs(&trusted(), Some("  ".into())).is_err());
    }

    #[test]
    fn config_without_endpoints_is_refused() {
        let cfg = config(json!({ "publicKeys": ["K"] }));
        assert!(check_inputs(&cfg, Some("1.0.0".into())).is_err());
    }

    #[test]
    fn caller_args_cannot_override_version_or_endpoints() {
        // `CheckInputs` is built without looking at the IPC args at all; the
        // hostile fields below are simply never read.
        let _hostile = json!({
            "currentVersion": "0.0.0",
            "endpoints": ["https://evil.example/old.json"],
            "endpoint": "https://evil.example/old.json",
            "manifestSignatureUrl": "https://evil.example/old.sig",
        });
        let inputs = check_inputs(&trusted(), Some("2.0.0".into())).unwrap();
        assert_eq!(inputs.current, "2.0.0");
        assert!(inputs
            .templates
            .iter()
            .all(|template| !template.contains("evil")));
    }

    #[test]
    fn every_mirror_uses_its_own_signature_url() {
        let urls = signature_urls(&["https://a/m.json".into(), "https://b/m.json".into()]);
        assert_eq!(urls, ["https://a/m.json.sig", "https://b/m.json.sig"]);
    }

    #[test]
    fn mirror_fallback_continues_with_its_own_signature() {
        let templates = vec!["https://b".to_string(), "https://a".to_string()];
        let sigs = signature_urls(&templates);
        let mut tried = Vec::new();
        let result = try_endpoints(&templates, |index, endpoint| {
            tried.push((endpoint.to_string(), sigs[index].clone()));
            if endpoint == "https://b" {
                Err(FetchFailure::Retryable("b is down".into()))
            } else {
                Ok(())
            }
        });
        assert_eq!(result.unwrap().1, 1);
        assert_eq!(
            tried,
            [
                ("https://b".to_string(), "https://b.sig".to_string()),
                ("https://a".to_string(), "https://a.sig".to_string()),
            ]
        );
    }

    #[test]
    fn untagged_manifest_is_the_default_channel() {
        let untagged = json!({ "version": "2.0.0" });
        let beta = json!({ "channel": "beta" });
        // Default config (explicit channel "stable") + manifest published
        // without a channel: the update must be accepted.
        assert!(check_manifest_channel(&untagged, Some("stable")).is_ok());
        // An install on another explicit channel still refuses it.
        assert!(check_manifest_channel(&untagged, Some("beta")).is_err());
        assert!(check_manifest_channel(&beta, Some("beta")).is_ok());
        assert!(check_manifest_channel(&json!({ "channel": "stable" }), Some("beta")).is_err());
        assert!(check_manifest_channel(&untagged, None).is_ok());
        assert!(check_manifest_channel(&beta, None).is_ok());
    }

    #[test]
    fn renders_templates_for_this_platform() {
        let (target, arch) = platform_parts();
        let urls = render_endpoints(
            &[
                "https://cdn/{{channel}}/{{target}}-{{arch}}/{{current_version}}/m.json".into(),
                "https://mirror/m.json".into(),
            ],
            "beta",
            "1.0.0",
        );
        assert_eq!(
            urls[0],
            format!("https://cdn/beta/{target}-{arch}/1.0.0/m.json")
        );
        assert_eq!(urls[1], "https://mirror/m.json");
    }

    #[test]
    fn manifest_channel_must_match_explicit_channel() {
        let beta = json!({ "channel": "beta" });
        assert!(check_manifest_channel(&beta, Some("beta")).is_ok());
        assert!(check_manifest_channel(&beta, Some("stable")).is_err());
        // No explicit client channel: any manifest is accepted.
        assert!(check_manifest_channel(&beta, None).is_ok());
        assert!(check_manifest_channel(&json!({}), None).is_ok());
        // A manifest without a channel counts as the default channel.
        assert!(check_manifest_channel(&json!({}), Some("stable")).is_ok());
        assert!(check_manifest_channel(&json!({}), Some("beta")).is_err());
    }

    #[test]
    fn rollout_is_decided_by_stable_bucket() {
        let version = "2.0.0";
        assert!(rollout_includes_install(None, "id", version).unwrap());
        assert!(rollout_includes_install(Some(&Value::Null), "id", version).unwrap());
        assert!(rollout_includes_install(Some(&json!(100)), "id", version).unwrap());
        assert!(!rollout_includes_install(Some(&json!(0)), "id", version).unwrap());
        assert!(rollout_includes_install(Some(&json!(101)), "id", version).is_err());
        assert!(rollout_includes_install(Some(&json!("50")), "id", version).is_err());

        // Same install, same release: same answer every time; 25% admits ~a quarter.
        let admitted = (0..2000)
            .filter(|i| {
                let id = format!("install-{i}");
                let first = rollout_includes_install(Some(&json!(25)), &id, version).unwrap();
                assert_eq!(
                    first,
                    rollout_includes_install(Some(&json!(25)), &id, version).unwrap()
                );
                first
            })
            .count();
        assert!((400..600).contains(&admitted), "admitted {admitted}");
    }
}
