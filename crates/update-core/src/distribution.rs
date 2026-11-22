//! Pure helpers for update distribution: release channels, endpoint URL
//! templating, mirror fallback, key rotation and staged rollout.
//!
//! Everything here is side-effect free so the host and the tooling share one
//! tested implementation.

/// Channel used for `{{channel}}` when the app never configured one.
pub const DEFAULT_CHANNEL: &str = "stable";

const MAX_CHANNEL_LEN: usize = 64;

/// Values substituted into an endpoint template.
#[derive(Debug, Clone, Copy)]
pub struct TemplateVars<'a> {
    pub channel: &'a str,
    /// Operating system part of the platform key (`windows`, `darwin`, `linux`).
    pub target: &'a str,
    /// CPU architecture part of the platform key (`x86_64`, `aarch64`).
    pub arch: &'a str,
    pub current_version: &'a str,
}

/// Validate a release channel name: 1..=64 chars of `[A-Za-z0-9._-]`, not
/// starting with `.` (keeps it a safe URL path segment and file name).
pub fn validate_channel(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("channel must not be empty".into());
    }
    if name.len() > MAX_CHANNEL_LEN {
        return Err(format!("channel exceeds {MAX_CHANNEL_LEN} characters"));
    }
    if name.starts_with('.') {
        return Err("channel must not start with '.'".into());
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "channel contains invalid character '{bad}'; allowed: letters, digits, '.', '_', '-'"
        ));
    }
    Ok(())
}

/// Percent-encode everything except RFC 3986 unreserved characters.
fn encode_component(value: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Replace `{{channel}}`, `{{target}}`, `{{arch}}` and `{{current_version}}`
/// (inner whitespace tolerated) in `template`. Values are percent-encoded.
/// Unknown placeholders are left untouched.
pub fn render_endpoint(template: &str, vars: &TemplateVars<'_>) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let value = match after[..end].trim() {
            "channel" => Some(vars.channel),
            "target" => Some(vars.target),
            "arch" => Some(vars.arch),
            "current_version" => Some(vars.current_version),
            _ => None,
        };
        match value {
            Some(value) => out.push_str(&encode_component(value)),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Why a single endpoint attempt failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchFailure {
    /// Network error, timeout or retryable HTTP status: try the next mirror.
    Retryable(String),
    /// Anything else (policy refusal, 4xx, bad signature): stop immediately.
    Fatal(String),
}

impl FetchFailure {
    pub fn into_message(self) -> String {
        match self {
            Self::Retryable(m) | Self::Fatal(m) => m,
        }
    }
}

/// `true` for HTTP statuses where another mirror may succeed: 5xx, 408, 429.
pub const fn is_retryable_status(status: u16) -> bool {
    status >= 500 || status == 408 || status == 429
}

/// Try `endpoints` in order. A [`FetchFailure::Retryable`] moves on to the next
/// endpoint; a [`FetchFailure::Fatal`] aborts. Returns the value and the index
/// of the endpoint that served it. When every endpoint fails retryably, the
/// error lists each failure in order.
pub fn try_endpoints<T>(
    endpoints: &[String],
    mut attempt: impl FnMut(usize, &str) -> Result<T, FetchFailure>,
) -> Result<(T, usize), String> {
    if endpoints.is_empty() {
        return Err("no updater endpoint configured".into());
    }
    let mut failures = Vec::new();
    for (index, endpoint) in endpoints.iter().enumerate() {
        match attempt(index, endpoint) {
            Ok(value) => return Ok((value, index)),
            Err(FetchFailure::Fatal(message)) => return Err(message),
            Err(FetchFailure::Retryable(message)) => failures.push(message),
        }
    }
    if failures.len() == 1 {
        return Err(failures.remove(0));
    }
    Err(format!(
        "all {} updater endpoints failed: {}",
        failures.len(),
        failures.join("; ")
    ))
}

/// Index of the first key for which `verify` succeeds (key rotation: a
/// signature is valid when any trusted key verifies it).
pub fn first_verifying_key<K>(keys: &[K], verify: impl FnMut(&K) -> bool) -> Option<usize> {
    keys.iter().position(verify)
}

/// Split a compiled-in pin (`TYND_UPDATER_PUBKEY`) into individual keys.
/// Keys are separated by commas, semicolons or whitespace.
pub fn split_key_list(pin: &str) -> Vec<&str> {
    pin.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|key| !key.is_empty())
        .collect()
}

/// Stable per-install rollout bucket in `0..100`.
///
/// Derived from the persisted random `install_id` and the release `version`
/// (FNV-1a 64), so an install keeps the same bucket for a given release while
/// different releases sample different cohorts.
pub fn rollout_bucket(install_id: &str, version: &str) -> u8 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in install_id
        .bytes()
        .chain(std::iter::once(0))
        .chain(version.trim().trim_start_matches('v').bytes())
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    // Finalise (avalanche) so low bits are well distributed before `% 100`.
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    u8::try_from(hash % 100).unwrap_or(0)
}

/// Decide whether an install receives a release rolled out to `rollout`
/// percent. `None` (field absent) means everyone. A value outside `0..=100`
/// is rejected.
pub fn rollout_allows(rollout: Option<f64>, bucket: u8) -> Result<bool, String> {
    let Some(percent) = rollout else {
        return Ok(true);
    };
    if !(0.0..=100.0).contains(&percent) {
        return Err(format!("manifest rollout {percent} is outside 0..=100"));
    }
    Ok(f64::from(bucket) < percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARS: TemplateVars<'static> = TemplateVars {
        channel: "beta",
        target: "windows",
        arch: "x86_64",
        current_version: "1.2.3-rc.1+build5",
    };

    #[test]
    fn renders_all_placeholders() {
        let url = render_endpoint(
            "https://cdn.example/{{channel}}/{{target}}-{{arch}}/{{current_version}}/manifest.json",
            &VARS,
        );
        assert_eq!(
            url,
            "https://cdn.example/beta/windows-x86_64/1.2.3-rc.1%2Bbuild5/manifest.json"
        );
    }

    #[test]
    fn tolerates_whitespace_and_repeats() {
        assert_eq!(
            render_endpoint("{{ channel }}/{{channel}}", &VARS),
            "beta/beta"
        );
    }

    #[test]
    fn leaves_unknown_and_unterminated_placeholders() {
        assert_eq!(
            render_endpoint("a/{{nope}}/{{channel", &VARS),
            "a/{{nope}}/{{channel"
        );
        assert_eq!(render_endpoint("plain", &VARS), "plain");
    }

    #[test]
    fn encodes_values_so_they_stay_one_path_segment() {
        let vars = TemplateVars {
            channel: "a/b?c",
            ..VARS
        };
        assert_eq!(render_endpoint("/{{channel}}/", &vars), "/a%2Fb%3Fc/");
    }

    #[test]
    fn validates_channels() {
        assert!(validate_channel("stable").is_ok());
        assert!(validate_channel("beta-2.1_x").is_ok());
        assert!(validate_channel("").is_err());
        assert!(validate_channel(".hidden").is_err());
        assert!(validate_channel("a/b").is_err());
        assert!(validate_channel("sp ace").is_err());
        assert!(validate_channel(&"x".repeat(65)).is_err());
    }

    fn endpoints() -> Vec<String> {
        vec!["a".into(), "b".into(), "c".into()]
    }

    #[test]
    fn falls_back_in_order_on_retryable_failures() {
        let mut seen = Vec::new();
        let result = try_endpoints(&endpoints(), |index, endpoint| {
            seen.push(endpoint.to_string());
            if index < 2 {
                Err(FetchFailure::Retryable(format!("{endpoint} down")))
            } else {
                Ok("manifest")
            }
        });
        assert_eq!(result, Ok(("manifest", 2)));
        assert_eq!(seen, ["a", "b", "c"]);
    }

    #[test]
    fn stops_at_first_success() {
        let mut calls = 0;
        let result = try_endpoints(&endpoints(), |_, _| {
            calls += 1;
            Ok(())
        });
        assert_eq!(result, Ok(((), 0)));
        assert_eq!(calls, 1);
    }

    #[test]
    fn fatal_failure_does_not_fall_back() {
        let mut calls = 0;
        let result: Result<((), usize), String> = try_endpoints(&endpoints(), |_, _| {
            calls += 1;
            Err(FetchFailure::Fatal("bad signature".into()))
        });
        assert_eq!(result, Err("bad signature".into()));
        assert_eq!(calls, 1);
    }

    #[test]
    fn aggregates_when_every_endpoint_fails() {
        let result: Result<((), usize), String> = try_endpoints(&endpoints(), |_, endpoint| {
            Err(FetchFailure::Retryable(format!("{endpoint} 503")))
        });
        assert_eq!(
            result,
            Err("all 3 updater endpoints failed: a 503; b 503; c 503".into())
        );
        let single: Result<((), usize), String> = try_endpoints(&["only".to_string()], |_, _| {
            Err(FetchFailure::Retryable("only 503".into()))
        });
        assert_eq!(single, Err("only 503".into()));
        assert!(try_endpoints::<()>(&[], |_, _| Ok(())).is_err());
    }

    #[test]
    fn classifies_statuses() {
        for status in [500, 502, 503, 504, 408, 429] {
            assert!(is_retryable_status(status), "{status}");
        }
        for status in [200, 301, 400, 401, 403, 404] {
            assert!(!is_retryable_status(status), "{status}");
        }
    }

    #[test]
    fn any_key_may_verify() {
        let keys = ["old", "new", "newer"];
        assert_eq!(first_verifying_key(&keys, |k| *k == "new"), Some(1));
        assert_eq!(first_verifying_key(&keys, |k| *k == "old"), Some(0));
        assert_eq!(first_verifying_key(&keys, |_| false), None);
        assert_eq!(first_verifying_key::<&str>(&[], |_| true), None);
    }

    #[test]
    fn splits_key_lists() {
        assert_eq!(split_key_list("a, b;c\n d"), ["a", "b", "c", "d"]);
        assert_eq!(split_key_list("single"), ["single"]);
        assert_eq!(split_key_list(" , "), Vec::<&str>::new());
    }

    #[test]
    fn bucket_is_stable_and_in_range() {
        for i in 0..500 {
            let id = format!("install-{i}");
            let bucket = rollout_bucket(&id, "2.0.0");
            assert!(bucket < 100);
            assert_eq!(bucket, rollout_bucket(&id, "v2.0.0"));
        }
        assert_ne!(
            (0..50)
                .map(|i| rollout_bucket(&format!("i{i}"), "1.0.0"))
                .collect::<Vec<_>>(),
            (0..50)
                .map(|i| rollout_bucket(&format!("i{i}"), "1.0.1"))
                .collect::<Vec<_>>(),
            "different releases should sample different cohorts"
        );
    }

    #[test]
    fn bucket_distribution_is_roughly_uniform() {
        let total = 10_000;
        let below_ten = (0..total)
            .filter(|i| rollout_bucket(&format!("id-{i}"), "3.1.4") < 10)
            .count();
        // Expect ~10% (1000); allow generous slack.
        assert!((800..1200).contains(&below_ten), "got {below_ten}");
    }

    #[test]
    fn rollout_gate() {
        assert_eq!(rollout_allows(None, 99), Ok(true));
        assert_eq!(rollout_allows(Some(100.0), 99), Ok(true));
        assert_eq!(rollout_allows(Some(0.0), 0), Ok(false));
        assert_eq!(rollout_allows(Some(10.0), 9), Ok(true));
        assert_eq!(rollout_allows(Some(10.0), 10), Ok(false));
        assert!(rollout_allows(Some(101.0), 0).is_err());
        assert!(rollout_allows(Some(-1.0), 0).is_err());
        assert!(rollout_allows(Some(f64::NAN), 0).is_err());
    }
}
