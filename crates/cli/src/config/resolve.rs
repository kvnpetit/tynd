use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;

use crate::fsx::absolute;

use super::{
    model::{
        EffectiveHostPlan, ObservabilityConfig, ResolvedUpdaterConfig, ResolvedUpdaterNetwork,
        TyndConfig, CORE_HOST_APIS, KNOWN_APIS,
    },
    validate::{
        expand_env, header_value_problem, parse_byte_size, parse_interval, MAX_OBSERVABILITY_BYTES,
    },
};

pub(crate) fn resolve_plugins(config: &TyndConfig) -> Vec<String> {
    config
        .plugins
        .iter()
        .map(|plugin| plugin.package().to_owned())
        .collect()
}

/// Channel used when the updater config leaves `channel` unset.
pub(crate) const DEFAULT_UPDATER_CHANNEL: &str = "stable";

/// `list` trimmed, blanks and repeats dropped.
fn trimmed_unique(list: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = Vec::with_capacity(list.len());
    for value in list {
        let value = value.trim();
        if !value.is_empty() && !merged.iter().any(|existing| existing == value) {
            merged.push(value.to_owned());
        }
    }
    merged
}

pub(crate) fn resolve_updater_config(config: &TyndConfig) -> Result<Option<ResolvedUpdaterConfig>> {
    let Some(updater) = &config.updater else {
        return Ok(None);
    };
    let endpoints = trimmed_unique(&updater.endpoints);
    let public_keys = trimmed_unique(&updater.public_keys);
    if endpoints.is_empty() {
        bail!("updater needs at least one endpoint");
    }
    if public_keys.is_empty() {
        bail!("updater needs at least one public key");
    }
    Ok(Some(ResolvedUpdaterConfig {
        endpoints,
        public_keys,
        channel: updater
            .channel
            .clone()
            .unwrap_or_else(|| DEFAULT_UPDATER_CHANNEL.to_owned()),
        prefer_patch: updater.prefer_patch,
        allow_patch: updater.allow_patch,
        max_patch_size: updater
            .max_patch_size
            .as_ref()
            .map(parse_byte_size)
            .transpose()?,
        fallback_to_full: updater.fallback_to_full,
        auto_check: updater.auto_check,
        check_interval_secs: parse_interval(&updater.check_interval)?,
        check_on_startup: updater.check_on_startup,
        auto_download: updater.auto_download,
        auto_apply: updater.auto_apply,
        prompt_style: updater.prompt_style,
        require_signature: updater.require_signature,
        require_major_prompt: updater.require_major_prompt,
    }))
}

/// Backend-only `proxy` / `headers` of the updater, with `${env:NAME}` expanded
/// through `lookup` (the process environment, then the `.env` files). `None`
/// when the config sets neither. Errors name the variable or header, never the
/// value.
pub(crate) fn resolve_updater_network(
    config: &TyndConfig,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Result<Option<ResolvedUpdaterNetwork>> {
    let Some(updater) = &config.updater else {
        return Ok(None);
    };
    let proxy = updater
        .proxy
        .as_deref()
        .map(str::trim)
        .filter(|proxy| !proxy.is_empty())
        .map(str::to_owned);
    let mut headers = BTreeMap::new();
    for (name, value) in &updater.headers {
        let expanded = expand_env(value, &mut lookup)
            .map_err(|problem| anyhow!("updater.headers.{name} {problem}"))?;
        if let Some(problem) = header_value_problem(&expanded) {
            bail!("updater.headers.{name} {problem}");
        }
        headers.insert(name.clone(), expanded);
    }
    if proxy.is_none() && headers.is_empty() {
        return Ok(None);
    }
    Ok(Some(ResolvedUpdaterNetwork { proxy, headers }))
}

/// Value of the `TYND_UPDATER_PUBKEY` build variable: every trusted key joined
/// with `,` (the host accepts `,`, `;` or whitespace separators).
pub(crate) fn updater_pubkey_pin(config: &TyndConfig) -> Option<String> {
    let updater = config.updater.as_ref()?;
    let keys = trimmed_unique(&updater.public_keys);
    (!keys.is_empty()).then(|| keys.join(","))
}

/// The observability pipelines that can actually run: a pipeline with a blank
/// DSN / endpoint is disabled, and `None` means nothing is active.
pub(crate) fn resolve_observability(config: &TyndConfig) -> Option<ObservabilityConfig> {
    let observability = config.observability.as_ref()?;
    let active = ObservabilityConfig {
        sentry: observability
            .sentry
            .clone()
            .filter(|sentry| !sentry.dsn.trim().is_empty()),
        otel: observability
            .otel
            .clone()
            .filter(|otel| !otel.endpoint.trim().is_empty()),
    };
    (active.sentry.is_some() || active.otel.is_some()).then_some(active)
}

/// JSON for the packed `observability.json` entry / dev `TYND_OBSERVABILITY`.
pub(crate) fn resolve_observability_json(config: &TyndConfig) -> Result<Option<String>> {
    let Some(active) = resolve_observability(config) else {
        return Ok(None);
    };
    let json = serde_json::to_string(&active).context("serialize observability config")?;
    ensure_observability_size(&json)?;
    Ok(Some(json))
}

fn ensure_observability_size(json: &str) -> Result<()> {
    if json.len() > MAX_OBSERVABILITY_BYTES {
        bail!("observability config is larger than {MAX_OBSERVABILITY_BYTES} bytes");
    }
    Ok(())
}

/// Host plan of an app: every app runs on a host compiled for it, with the
/// core APIs plus the ones the config and its plugins request.
pub(crate) fn resolve_effective_host_plan(
    config: &TyndConfig,
    cwd: &Path,
) -> Result<EffectiveHostPlan> {
    let mut apis: BTreeSet<String> = CORE_HOST_APIS.iter().map(|api| (*api).to_owned()).collect();
    apis.extend(config.apis.iter().cloned());
    if config.updater.is_some() {
        apis.insert("updater".to_owned());
    }

    for entry in &config.plugins {
        apis.extend(entry.configured_apis());
        let manifest = resolve_plugin_manifest(entry.package(), cwd)?
            .with_context(|| format!("plugin package {} is not installed", entry.package()))?;
        if let Some(declared) = manifest.pointer("/tynd/apis").and_then(Value::as_array) {
            for api in declared.iter().filter_map(Value::as_str) {
                if !KNOWN_APIS.contains(&api) {
                    bail!(
                        "plugin \"{}\" declares unknown tynd.apis entry \"{api}\"",
                        entry.package()
                    );
                }
                apis.insert(api.to_owned());
            }
        }
    }

    Ok(EffectiveHostPlan {
        apis: apis.into_iter().collect(),
    })
}

pub(crate) fn resolve_backend_plugins(config: &TyndConfig, cwd: &Path) -> Result<Vec<String>> {
    let mut packages = Vec::new();
    for entry in &config.plugins {
        let manifest = resolve_plugin_manifest(entry.package(), cwd)?
            .with_context(|| format!("plugin package {} is not installed", entry.package()))?;
        let native = manifest
            .pointer("/tynd/native")
            .is_some_and(|value| !value.is_null());
        let backend = manifest
            .pointer("/tynd/backend")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || manifest
                .get("exports")
                .and_then(Value::as_object)
                .is_some_and(|exports| exports.contains_key("./backend"));
        if !native && backend {
            packages.push(entry.package().to_owned());
        }
    }
    Ok(packages)
}

fn resolve_plugin_manifest(package: &str, cwd: &Path) -> Result<Option<Value>> {
    let mut directory = absolute(cwd)?;
    loop {
        let direct = directory
            .join("node_modules")
            .join(package)
            .join("package.json");
        if direct.is_file() {
            return read_manifest(&direct).map(Some);
        }
        for workspace in ["packages", "plugins/host", "plugins/standalone"] {
            let workspace = directory.join(workspace);
            let Ok(entries) = fs::read_dir(&workspace) else {
                continue;
            };
            for entry in entries.flatten() {
                if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let manifest = entry.path().join("package.json");
                if !manifest.is_file() {
                    continue;
                }
                let Ok(value) = read_manifest(&manifest) else {
                    continue;
                };
                if value.get("name").and_then(Value::as_str) == Some(package) {
                    return Ok(Some(value));
                }
            }
        }
        if !directory.pop() {
            return Ok(None);
        }
    }
}

fn read_manifest(path: &Path) -> Result<Value> {
    let source = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("parse {}", path.display()))
}

#[cfg(test)]
mod tests;
