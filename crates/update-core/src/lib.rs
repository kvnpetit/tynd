#![forbid(unsafe_code)]

use serde::Deserialize;

mod distribution;

pub use distribution::{
    first_verifying_key, is_retryable_status, render_endpoint, rollout_allows, rollout_bucket,
    split_key_list, try_endpoints, validate_channel, FetchFailure, TemplateVars, DEFAULT_CHANNEL,
};

/// Returns `true` if `latest` is strictly greater than `current` according to
/// full SemVer semantics.
pub fn is_update_available(current: &str, latest: &str) -> anyhow::Result<bool> {
    let current_v = semver::Version::parse(current.trim().trim_start_matches('v'))
        .map_err(|e| anyhow::anyhow!("invalid current version '{current}': {e}"))?;
    let latest_v = semver::Version::parse(latest.trim().trim_start_matches('v'))
        .map_err(|e| anyhow::anyhow!("invalid latest version '{latest}': {e}"))?;
    Ok(latest_v > current_v)
}

/// Returns `true` if `version` satisfies the semver requirement string `req`.
pub fn meets_requirement(version: &str, req: &str) -> anyhow::Result<bool> {
    let v = semver::Version::parse(version.trim().trim_start_matches('v'))
        .map_err(|e| anyhow::anyhow!("invalid version '{version}': {e}"))?;
    let r = semver::VersionReq::parse(req)
        .map_err(|e| anyhow::anyhow!("invalid version requirement '{req}': {e}"))?;
    Ok(r.matches(&v))
}

/// Compare two version strings, accepting optional leading `v`.
pub fn version_eq(a: &str, b: &str) -> bool {
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

/// Constant-time equality on hex strings of the same length.
pub fn hashes_match(expected: &str, actual: &str) -> bool {
    let exp = expected.as_bytes();
    let act = actual.as_bytes();
    if exp.len() != act.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in exp.iter().zip(act.iter()) {
        diff |= a.to_ascii_lowercase() ^ b.to_ascii_lowercase();
    }
    diff == 0
}

const fn default_true() -> bool {
    true
}

fn default_check_interval_secs() -> u64 {
    12 * 60 * 60
}

fn default_prompt_style() -> PromptStyle {
    PromptStyle::Modal
}

/// How the updater UI should surface a pending update to the user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PromptStyle {
    #[default]
    Modal,
    Tray,
    Silent,
    Manual,
}

impl PromptStyle {
    pub const fn as_ref(self) -> &'static str {
        match self {
            Self::Modal => "modal",
            Self::Tray => "tray",
            Self::Silent => "silent",
            Self::Manual => "manual",
        }
    }
}

/// User-tunable updater preferences. Mirrors the CLI config schema.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)] // Public camelCase config mirrors the TypeScript schema.
pub struct UpdaterConfig {
    /// Mirrors tried in order (network / 5xx fallback). May contain the
    /// `{{channel}}`, `{{target}}`, `{{arch}}`, `{{current_version}}` placeholders.
    #[serde(default)]
    pub endpoints: Vec<String>,
    /// Trusted verifying keys; a signature is valid when any of them verifies
    /// it (key rotation).
    #[serde(default)]
    pub public_keys: Vec<String>,
    /// Default release channel (`stable` when unset). Overridable at runtime.
    #[serde(default)]
    pub channel: Option<String>,

    #[serde(default = "default_true")]
    pub prefer_patch: bool,
    #[serde(default = "default_true")]
    pub allow_patch: bool,
    #[serde(default)]
    pub max_patch_size: Option<u64>,
    #[serde(default = "default_true")]
    pub fallback_to_full: bool,

    #[serde(default = "default_true")]
    pub auto_check: bool,
    #[serde(default = "default_check_interval_secs")]
    pub check_interval_secs: u64,
    #[serde(default = "default_true")]
    pub check_on_startup: bool,
    #[serde(default = "default_true")]
    pub auto_download: bool,
    #[serde(default)]
    pub auto_apply: bool,
    #[serde(default = "default_prompt_style")]
    pub prompt_style: PromptStyle,

    #[serde(default = "default_true")]
    pub require_signature: bool,
    #[serde(default = "default_true")]
    pub require_major_prompt: bool,
}

impl UpdaterConfig {
    pub fn normalised(mut self) -> Self {
        if self.check_interval_secs < 60 {
            self.check_interval_secs = 60;
        }
        if matches!(self.max_patch_size, Some(0)) {
            self.max_patch_size = None;
        }
        self
    }

    /// Endpoints to try, in order. Blank and duplicate entries are dropped.
    pub fn resolved_endpoints(&self) -> Vec<String> {
        trimmed_unique(&self.endpoints)
    }

    /// Trusted verifying keys. Blank and duplicate entries are dropped.
    pub fn resolved_public_keys(&self) -> Vec<String> {
        trimmed_unique(&self.public_keys)
    }

    /// Check the distribution fields: at least one endpoint and key, and a
    /// well-formed channel.
    pub fn validate_distribution(&self) -> Result<(), String> {
        if self.resolved_endpoints().is_empty() {
            return Err("updater needs at least one endpoint".into());
        }
        if self.resolved_public_keys().is_empty() {
            return Err("updater needs at least one public key".into());
        }
        if let Some(channel) = &self.channel {
            validate_channel(channel).map_err(|e| format!("updater channel: {e}"))?;
        }
        Ok(())
    }
}

fn trimmed_unique(list: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for value in list {
        let value = value.trim();
        if !value.is_empty() && !out.iter().any(|existing| existing == value) {
            out.push(value.to_string());
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchCompression {
    Zstd,
    None,
}

impl PatchCompression {
    pub fn from_manifest_or_url(value: Option<&str>, url: &str) -> Result<Self, String> {
        match value {
            Some("zstd") => Ok(Self::Zstd),
            Some("none") => Ok(Self::None),
            Some(other) => Err(format!(
                "unknown compression '{other}'; expected 'zstd' or 'none'"
            )),
            None => {
                let path_end = url.find(['?', '#']).unwrap_or(url.len());
                let extension = url[..path_end]
                    .rsplit_once('.')
                    .map(|(_, extension)| extension);
                if extension.is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("zst") || extension.eq_ignore_ascii_case("zstd")
                }) {
                    Ok(Self::Zstd)
                } else {
                    Ok(Self::None)
                }
            },
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zstd => "zstd",
            Self::None => "none",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> UpdaterConfig {
        serde_json::from_str(json).expect("config parses")
    }

    #[test]
    fn lists_drop_blank_and_duplicate_entries() {
        let cfg = parse(
            r#"{"endpoints":["https://b","https://a"," ","https://b"],
                "publicKeys":["K2","K1","K2"],"channel":"beta"}"#,
        );
        assert_eq!(cfg.resolved_endpoints(), ["https://b", "https://a"]);
        assert_eq!(cfg.resolved_public_keys(), ["K2", "K1"]);
        assert_eq!(cfg.channel.as_deref(), Some("beta"));
        assert!(cfg.validate_distribution().is_ok());
    }

    #[test]
    fn rejects_missing_endpoint_key_or_bad_channel() {
        assert!(parse(r#"{"publicKeys":["K"]}"#)
            .validate_distribution()
            .is_err());
        assert!(parse(r#"{"endpoints":["https://a"]}"#)
            .validate_distribution()
            .is_err());
        assert!(parse(r#"{"endpoint":"https://a","publicKey":"K"}"#)
            .validate_distribution()
            .is_err());
        assert!(
            parse(r#"{"endpoints":["https://a"],"publicKeys":["K"],"channel":"a/b"}"#)
                .validate_distribution()
                .is_err()
        );
    }
}
