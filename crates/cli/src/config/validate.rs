use std::{collections::HashSet, path::Path};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use regex::Regex;

use super::model::{
    permission_api, BundleConfig, ByteSize, ConfigError, FileAssociation, ObservabilityConfig,
    PluginEntry, TyndConfig, UpdaterConfig, WebView2InstallMode, KNOWN_APIS,
};

/// Placeholders the updater expands in an endpoint template.
const ENDPOINT_PLACEHOLDERS: &[&str] = &[
    "{{channel}}",
    "{{target}}",
    "{{arch}}",
    "{{current_version}}",
];
/// Longest channel name accepted (mirrors `tynd_update_core::validate_channel`).
const MAX_CHANNEL_LEN: usize = 64;
/// Limits and reserved names of `updater.headers` (mirror the host's
/// `os/updater/network.rs`).
const MAX_UPDATER_HEADERS: usize = 32;
const MAX_UPDATER_HEADER_VALUE_BYTES: usize = 4096;
const FORBIDDEN_UPDATER_HEADERS: [&str; 12] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "proxy-authorization",
    "proxy-connection",
    "proxy-authenticate",
];
/// Size cap of the packed `observability.json` entry (`MAX_BOOT_METADATA_BYTES`).
pub(super) const MAX_OBSERVABILITY_BYTES: usize = 4096;

pub(super) fn validate_config_at(config: &TyndConfig, path: Option<&Path>) -> Result<()> {
    let mut issues = Vec::new();
    if path_is_empty(&config.backend) {
        issues.push("backend must not be empty".to_owned());
    }
    if path_is_empty(&config.frontend_dir) {
        issues.push("frontendDir must not be empty".to_owned());
    }
    if config
        .dev_url
        .as_deref()
        .is_some_and(|value| !is_http_url(value))
    {
        issues.push("devUrl must be an http(s) URL with a host".to_owned());
    }

    validate_apis("apis", &config.apis, &mut issues);
    validate_plugins(&config.plugins, &mut issues);
    validate_protocols("protocols", &config.protocols, &mut issues);
    let mut sidecars = HashSet::new();
    for (index, sidecar) in config.sidecars.iter().enumerate() {
        if !safe_sidecar_name(&sidecar.name) {
            issues.push(format!("sidecars.{index}.name is not a safe filename"));
        } else if !sidecars.insert(&sidecar.name) {
            issues.push(format!("sidecar name \"{}\" is duplicated", sidecar.name));
        }
        if path_is_empty(&sidecar.path) {
            issues.push(format!("sidecars.{index}.path must not be empty"));
        }
    }

    if let Some(bundle) = &config.bundle {
        validate_bundle(bundle, &mut issues);
    }
    if let Some(updater) = &config.updater {
        validate_updater(updater, &mut issues);
    }
    if let Some(observability) = &config.observability {
        validate_observability(observability, &mut issues);
    }

    if issues.is_empty() {
        Ok(())
    } else {
        Err(ConfigError::new(path, issues).into())
    }
}

fn validate_updater(updater: &UpdaterConfig, issues: &mut Vec<String>) {
    if updater.endpoints.is_empty() {
        issues.push("updater needs a non-empty endpoints list".into());
    }
    let mut seen = HashSet::new();
    for (index, endpoint) in updater.endpoints.iter().enumerate() {
        validate_updater_endpoint(&format!("updater.endpoints.{index}"), endpoint, issues);
        if !seen.insert(endpoint.as_str()) {
            issues.push(format!(
                "updater.endpoints contains duplicate \"{endpoint}\""
            ));
        }
    }

    if updater.public_keys.is_empty() {
        issues.push("updater needs a non-empty publicKeys list".into());
    }
    let mut seen = HashSet::new();
    for (index, key) in updater.public_keys.iter().enumerate() {
        if !is_ed25519_public_key(key) {
            issues.push(format!(
                "updater.publicKeys.{index} must be a base64-encoded 32-byte Ed25519 public key"
            ));
        } else if !seen.insert(key.as_str()) {
            issues.push(format!("updater.publicKeys.{index} is duplicated"));
        }
    }

    if let Some(channel) = &updater.channel {
        if let Some(problem) = channel_problem(channel) {
            issues.push(format!("updater.channel {problem}"));
        }
    }
    if let Some(proxy) = &updater.proxy {
        if let Some(problem) = proxy_problem(proxy) {
            issues.push(format!("updater.proxy {problem}"));
        }
    }
    validate_updater_headers(updater, issues);
    if !updater.require_signature {
        issues.push(
            "updater.requireSignature must be true; unsigned updates are not supported".into(),
        );
    }
    if parse_interval(&updater.check_interval).is_err() {
        issues.push("updater.checkInterval must look like 30s, 12h, 7d, or bare seconds".into());
    }
    if let Some(size) = &updater.max_patch_size {
        if parse_byte_size(size).is_err() {
            issues.push("updater.maxPatchSize must be bytes or a value such as 5MB".into());
        }
    }
}

/// An endpoint is an http(s) URL once the known placeholders are expanded;
/// any other `{{...}}` is a typo that would reach the wire verbatim.
fn validate_updater_endpoint(label: &str, endpoint: &str, issues: &mut Vec<String>) {
    let mut expanded = endpoint.to_owned();
    for placeholder in ENDPOINT_PLACEHOLDERS {
        expanded = expanded.replace(placeholder, "x");
    }
    if expanded.contains("{{") || expanded.contains("}}") {
        issues.push(format!(
            "{label} contains an unknown placeholder (supported: {})",
            ENDPOINT_PLACEHOLDERS.join(", ")
        ));
    } else if !is_http_url(&expanded) {
        issues.push(format!("{label} must be an http(s) URL"));
    }
}

/// Mirror of the host's proxy check (`os/updater/network.rs`): a plain
/// `http(s)://host[:port]` URL.
pub(super) fn proxy_problem(proxy: &str) -> Option<String> {
    let Ok(parsed) = url::Url::parse(proxy.trim()) else {
        return Some("is not a valid URL".into());
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return Some("must use the http or https scheme".into());
    }
    if !parsed.has_host() || parsed.query().is_some() || parsed.fragment().is_some() {
        return Some("must be a plain scheme://host[:port] URL".into());
    }
    None
}

fn validate_updater_headers(updater: &UpdaterConfig, issues: &mut Vec<String>) {
    if updater.headers.is_empty() {
        return;
    }
    if updater.headers.len() > MAX_UPDATER_HEADERS {
        issues.push(format!(
            "updater.headers allows at most {MAX_UPDATER_HEADERS} headers"
        ));
    }
    let mut seen = HashSet::new();
    for (name, value) in &updater.headers {
        if !is_header_name(name) {
            issues.push(format!(
                "updater.headers: \"{name}\" is not a valid header name"
            ));
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if FORBIDDEN_UPDATER_HEADERS.contains(&lower.as_str()) {
            issues.push(format!(
                "updater.headers: \"{name}\" is set by the HTTP client and cannot be configured"
            ));
        } else if !seen.insert(lower) {
            issues.push(format!("updater.headers: \"{name}\" is listed twice"));
        }
        // Syntax only: the value itself is never echoed (usually a credential).
        match expand_env(value, |_| Some("x".to_owned())) {
            Err(problem) => issues.push(format!("updater.headers.{name} {problem}")),
            Ok(expanded) => {
                if let Some(problem) = header_value_problem(&expanded) {
                    issues.push(format!("updater.headers.{name} {problem}"));
                }
            },
        }
    }
    let has_fixed_origin = updater
        .endpoints
        .iter()
        .any(|endpoint| endpoint_has_fixed_origin(endpoint));
    if !has_fixed_origin {
        issues.push(
            "updater.headers need an endpoint with a fixed origin (no placeholder in the \
             scheme, host or port)"
                .into(),
        );
    }
}

/// Headers are sent only to the origins of the endpoints, so at least one must
/// not depend on a placeholder (mirrors `TrustedNetwork::from_config`).
fn endpoint_has_fixed_origin(endpoint: &str) -> bool {
    url::Url::parse(endpoint).is_ok_and(|parsed| {
        let origin = parsed.origin();
        origin.is_tuple() && !origin.ascii_serialization().contains('{')
    })
}

/// RFC 9110 `token`.
fn is_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

/// Problem with an expanded header value, never quoting it.
pub(super) fn header_value_problem(value: &str) -> Option<String> {
    if value.len() > MAX_UPDATER_HEADER_VALUE_BYTES {
        return Some(format!(
            "is longer than {MAX_UPDATER_HEADER_VALUE_BYTES} bytes"
        ));
    }
    if value
        .chars()
        .any(|character| character.is_control() && character != '\t')
    {
        return Some("contains a control character".into());
    }
    None
}

/// Expand every `${env:NAME}` in `value` through `lookup`. A malformed
/// placeholder or an unset variable is an error that names the variable, never
/// the value.
pub(super) fn expand_env(
    value: &str,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Result<String, String> {
    const OPEN: &str = "${env:";
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find(OPEN) {
        output.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find('}') else {
            return Err("has an unterminated ${env:NAME} placeholder".into());
        };
        let name = &after[..end];
        let valid = name
            .chars()
            .next()
            .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
            && name
                .chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric());
        if !valid {
            return Err(
                "has an invalid ${env:NAME} placeholder (NAME is letters, digits, _)".into(),
            );
        }
        let Some(resolved) = lookup(name) else {
            return Err(format!("references unset environment variable {name}"));
        };
        output.push_str(&resolved);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

/// Mirror of `tynd_update_core::validate_channel`.
fn channel_problem(channel: &str) -> Option<String> {
    if channel.is_empty() {
        return Some("must not be empty".into());
    }
    if channel.len() > MAX_CHANNEL_LEN {
        return Some(format!("exceeds {MAX_CHANNEL_LEN} characters"));
    }
    if channel.starts_with('.') {
        return Some("must not start with '.'".into());
    }
    channel
        .chars()
        .find(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
        })
        .map(|bad| {
            format!("contains invalid character {bad:?}; allowed: letters, digits, '.', '_', '-'")
        })
}

fn validate_observability(observability: &ObservabilityConfig, issues: &mut Vec<String>) {
    if let Some(sentry) = &observability.sentry {
        if !sentry.dsn.trim().is_empty() && !is_http_url(sentry.dsn.trim()) {
            issues.push("observability.sentry.dsn must be an http(s) DSN URL or blank".into());
        }
        validate_optional_nonempty(
            "observability.sentry.environment",
            sentry.environment.as_deref(),
            issues,
        );
        validate_optional_nonempty(
            "observability.sentry.release",
            sentry.release.as_deref(),
            issues,
        );
        if sentry
            .sample_rate
            .is_some_and(|rate| !(0.0..=1.0).contains(&rate))
        {
            issues.push("observability.sentry.sampleRate must be between 0 and 1".into());
        }
    }
    if let Some(otel) = &observability.otel {
        if !otel.endpoint.trim().is_empty() && !is_http_url(otel.endpoint.trim()) {
            issues.push("observability.otel.endpoint must be an http(s) URL or blank".into());
        }
        validate_optional_nonempty(
            "observability.otel.serviceName",
            otel.service_name.as_deref(),
            issues,
        );
    }
    if serde_json::to_vec(observability).is_ok_and(|json| json.len() > MAX_OBSERVABILITY_BYTES) {
        issues.push(format!(
            "observability is larger than {MAX_OBSERVABILITY_BYTES} bytes once serialized"
        ));
    }
}

fn validate_plugins(plugins: &[PluginEntry], issues: &mut Vec<String>) {
    let mut packages = HashSet::new();
    for (index, entry) in plugins.iter().enumerate() {
        let package = entry.package();
        if package.trim().is_empty() {
            issues.push(format!("plugins.{index}.pkg must not be empty"));
        } else if !packages.insert(package) {
            issues.push(format!("plugin package \"{package}\" is duplicated"));
        }
        let PluginEntry::Detailed(plugin) = entry else {
            continue;
        };
        if !plugin.apis.is_empty() && !plugin.permissions.is_empty() {
            issues.push(format!(
                "plugin \"{}\" cannot declare both apis and permissions",
                plugin.pkg
            ));
        }
        validate_apis(&format!("plugins.{index}.apis"), &plugin.apis, issues);
        for permission in &plugin.permissions {
            if permission_api(permission).is_none() {
                issues.push(format!(
                    "plugins.{index}.permissions contains invalid selector \"{permission}\""
                ));
            }
        }
        for feature in &plugin.features {
            if feature.is_empty()
                || !feature
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
            {
                issues.push(format!(
                    "plugins.{index}.features contains invalid Cargo feature \"{feature}\""
                ));
            }
        }
    }
}
fn validate_bundle(bundle: &BundleConfig, issues: &mut Vec<String>) {
    if !is_reverse_dns(&bundle.identifier) {
        issues.push("bundle.identifier must be reverse-DNS (for example com.example.app)".into());
    }
    validate_optional_nonempty("bundle.displayName", bundle.display_name.as_deref(), issues);
    validate_nonempty_items("bundle.categories", &bundle.categories, issues);
    validate_nonempty_items("bundle.resources", &bundle.resources, issues);
    for (index, association) in bundle.file_associations.iter().enumerate() {
        validate_file_association(index, association, issues);
    }
    validate_linux_bundle(bundle, issues);
    validate_windows_bundle(bundle, issues);
    if let Some(schemes) = bundle
        .macos
        .as_ref()
        .and_then(|macos| macos.url_schemes.as_ref())
    {
        validate_protocol_syntax("bundle.macos.urlSchemes", schemes, issues);
    }
    validate_bundle_signing(bundle, issues);
}

fn validate_file_association(
    index: usize,
    association: &FileAssociation,
    issues: &mut Vec<String>,
) {
    let label = format!("bundle.fileAssociations.{index}");
    if association.ext.is_empty() {
        issues.push(format!("{label}.ext must contain at least one extension"));
    }
    for extension in &association.ext {
        if !valid_file_extension(extension) {
            issues.push(format!(
                "{label}.ext contains invalid extension {extension:?}"
            ));
        }
    }
    if let Some(content_types) = &association.content_types {
        validate_nonempty_items(&format!("{label}.contentTypes"), content_types, issues);
    }
    for (field, value) in [
        ("name", association.name.as_deref()),
        ("description", association.description.as_deref()),
        ("mimeType", association.mime_type.as_deref()),
    ] {
        validate_optional_nonempty(&format!("{label}.{field}"), value, issues);
    }
    if let Some(exported) = &association.exported_type {
        validate_optional_nonempty(
            &format!("{label}.exportedType.identifier"),
            Some(&exported.identifier),
            issues,
        );
        if let Some(conforms_to) = &exported.conforms_to {
            validate_nonempty_items(
                &format!("{label}.exportedType.conformsTo"),
                conforms_to,
                issues,
            );
        }
    }
}

fn validate_linux_bundle(bundle: &BundleConfig, issues: &mut Vec<String>) {
    if let Some(deb) = &bundle.deb {
        if let Some(depends) = &deb.depends {
            validate_nonempty_items("bundle.deb.depends", depends, issues);
        }
        validate_optional_nonempty("bundle.deb.section", deb.section.as_deref(), issues);
        validate_optional_nonempty("bundle.deb.priority", deb.priority.as_deref(), issues);
    }
    if let Some(requires) = bundle.rpm.as_ref().and_then(|rpm| rpm.requires.as_ref()) {
        validate_nonempty_items("bundle.rpm.requires", requires, issues);
    }
}

fn validate_windows_bundle(bundle: &BundleConfig, issues: &mut Vec<String>) {
    let Some(windows) = &bundle.windows else {
        return;
    };
    if let Some(version) = &windows.minimum_web_view2_version {
        let parts = version.split('.').collect::<Vec<_>>();
        if parts.len() != 4
            || parts.iter().any(|part| {
                part.is_empty() || !part.chars().all(|character| character.is_ascii_digit())
            })
        {
            issues.push(
                "bundle.windows.minimumWebView2Version must contain four numeric parts (for example 120.0.0.0)"
                    .into(),
            );
        }
    }
    if let Some(WebView2InstallMode::FixedRuntime { path }) = &windows.webview_install_mode {
        validate_optional_nonempty("bundle.windows.webviewInstallMode.path", Some(path), issues);
    }
}

fn validate_bundle_signing(bundle: &BundleConfig, issues: &mut Vec<String>) {
    let Some(sign) = &bundle.sign else {
        return;
    };
    if let Some(windows) = &sign.windows {
        validate_optional_nonempty(
            "bundle.sign.windows.certificate",
            Some(&windows.certificate),
            issues,
        );
        if windows
            .timestamp_url
            .as_deref()
            .is_some_and(|value| !is_http_url(value))
        {
            issues
                .push("bundle.sign.windows.timestampUrl must be an http(s) URL with a host".into());
        }
    }
    if let Some(macos) = &sign.macos {
        if path_is_empty(&macos.identity) {
            issues.push("bundle.sign.macos.identity must not be empty".into());
        }
        if let Some(notarize) = &macos.notarize {
            validate_optional_nonempty(
                "bundle.sign.macos.notarize.apiKey",
                Some(&notarize.api_key),
                issues,
            );
        }
    }
}
fn validate_optional_nonempty(label: &str, value: Option<&str>, issues: &mut Vec<String>) {
    if value.is_some_and(str::is_empty) {
        issues.push(format!("{label} must not be empty"));
    }
}

fn validate_nonempty_items(label: &str, values: &[String], issues: &mut Vec<String>) {
    for (index, value) in values.iter().enumerate() {
        if value.is_empty() {
            issues.push(format!("{label}.{index} must not be empty"));
        }
    }
}

fn valid_file_extension(extension: &str) -> bool {
    let extension = extension.strip_prefix('.').unwrap_or(extension);
    extension
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".+_-".contains(character))
}

pub(crate) fn parse_bytes(input: &str) -> Result<u64> {
    let value = input.trim();
    let pattern = Regex::new(r"(?i)^(\d+(?:\.\d+)?)\s*(b|kb|mb|gb|tb|kib|mib|gib|tib)?$")?;
    let captures = pattern
        .captures(value)
        .with_context(|| format!("cannot parse byte size \"{input}\""))?;
    let amount: f64 = captures[1].parse()?;
    let multiplier = match captures
        .get(2)
        .map(|value| value.as_str().to_ascii_lowercase())
        .as_deref()
        .unwrap_or("b")
    {
        "b" => 1_f64,
        "kb" => 1_000_f64,
        "mb" => 1_000_000_f64,
        "gb" => 1_000_000_000_f64,
        "tb" => 1_000_000_000_000_f64,
        "kib" => 1_024_f64,
        "mib" => 1_048_576_f64,
        "gib" => 1_073_741_824_f64,
        "tib" => 1_099_511_627_776_f64,
        _ => unreachable!(),
    };
    let bytes = amount * multiplier;
    if !bytes.is_finite() || bytes < 0.0 || bytes > u64::MAX as f64 {
        bail!("byte size is outside the supported range");
    }
    Ok(bytes.round() as u64)
}

pub(crate) fn parse_interval(input: &str) -> Result<u64> {
    let pattern = Regex::new(r"(?i)^(\d+)\s*(s|m|h|d|sec|secs|min|mins|hour|hours|day|days)?$")?;
    let captures = pattern
        .captures(input.trim())
        .with_context(|| format!("cannot parse interval \"{input}\""))?;
    let amount: u64 = captures[1].parse()?;
    let multiplier = match captures
        .get(2)
        .map(|value| value.as_str().to_ascii_lowercase())
        .as_deref()
        .unwrap_or("s")
    {
        "s" | "sec" | "secs" => 1,
        "m" | "min" | "mins" => 60,
        "h" | "hour" | "hours" => 3_600,
        "d" | "day" | "days" => 86_400,
        _ => unreachable!(),
    };
    amount
        .checked_mul(multiplier)
        .context("interval is outside the supported range")
}

pub(super) fn parse_byte_size(value: &ByteSize) -> Result<u64> {
    match value {
        ByteSize::Bytes(bytes) => Ok(*bytes),
        ByteSize::Human(value) => parse_bytes(value),
    }
}

fn validate_apis(label: &str, apis: &[String], issues: &mut Vec<String>) {
    let mut seen = HashSet::new();
    for api in apis {
        if !KNOWN_APIS.contains(&api.as_str()) {
            issues.push(format!("{label} contains unknown API \"{api}\""));
        } else if !seen.insert(api) {
            issues.push(format!("{label} contains duplicate API \"{api}\""));
        }
    }
}

fn validate_protocol_syntax(label: &str, protocols: &[String], issues: &mut Vec<String>) {
    for protocol in protocols {
        if !valid_protocol(protocol) {
            issues.push(format!("{label} contains invalid protocol \"{protocol}\""));
        }
    }
}

fn valid_protocol(protocol: &str) -> bool {
    protocol
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && protocol
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character))
}
fn validate_protocols(label: &str, protocols: &[String], issues: &mut Vec<String>) {
    const RESERVED: &[&str] = &[
        "http",
        "https",
        "file",
        "ftp",
        "mailto",
        "javascript",
        "data",
        "about",
        "blob",
        "tynd",
        "tynd-bin",
    ];
    let mut seen = HashSet::new();
    for protocol in protocols {
        if !valid_protocol(protocol) {
            issues.push(format!("{label} contains invalid protocol \"{protocol}\""));
        } else if RESERVED.contains(&protocol.to_ascii_lowercase().as_str()) {
            issues.push(format!("{label} contains reserved protocol \"{protocol}\""));
        } else if !seen.insert(protocol.to_ascii_lowercase()) {
            issues.push(format!(
                "{label} contains duplicate protocol \"{protocol}\""
            ));
        }
    }
}

fn safe_sidecar_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
}

fn is_reverse_dns(identifier: &str) -> bool {
    let segments: Vec<_> = identifier.split('.').collect();
    segments.len() >= 3
        && segments[0]
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
}

fn is_ed25519_public_key(value: &str) -> bool {
    let value = value.strip_prefix("ed25519:").unwrap_or(value);
    BASE64.decode(value).is_ok_and(|bytes| bytes.len() == 32)
}

fn is_http_url(value: &str) -> bool {
    url::Url::parse(value)
        .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
}

fn path_is_empty(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

#[cfg(test)]
mod tests;
