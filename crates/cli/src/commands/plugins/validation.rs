use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::fsx::read_json;
use crate::output::{step, success, warn, write_json, write_line};

use super::{
    shared::{native_crate_name, resolve_plugin_manifest, validate_native_crate_name, RegexCache},
    PluginCheck, PluginCheckRequest, PluginListRequest,
};

pub(in crate::commands) fn plugin_check(request: &PluginCheckRequest) -> Result<Vec<PluginCheck>> {
    let checks = collect_plugin_checks(&request.directory);
    let failures = checks.iter().filter(|check| check.status == "fail").count();
    let warnings = checks.iter().filter(|check| check.status == "warn").count();
    if request.json {
        write_json(&json!({
            "ok": failures == 0,
            "fails": failures,
            "warns": warnings,
            "issues": checks,
        }))?;
        if failures > 0 {
            bail!("{failures} plugin check(s) failed");
        }
        return Ok(checks);
    }
    for check in &checks {
        step(format!(
            "{:<26} {:<5} {}",
            check.name, check.status, check.detail
        ));
        if let Some(hint) = &check.hint {
            eprintln!("    -> {hint}");
        }
    }
    if failures > 0 {
        bail!("{failures} plugin check(s) failed, {warnings} warning(s)");
    }
    if warnings > 0 {
        warn(format!("{warnings} plugin warning(s)"));
    } else {
        success("Plugin layout OK");
    }
    Ok(checks)
}

pub(in crate::commands) fn plugin_list(
    request: &PluginListRequest,
    config: &crate::config::TyndConfig,
) -> Result<()> {
    let entries = crate::config::resolve_plugins(config)
        .into_iter()
        .map(|package| {
            let manifest = resolve_plugin_manifest(&request.root, &package);
            let kind = plugin_list_kind(manifest.as_ref());
            let family = manifest
                .as_ref()
                .and_then(|value| value.pointer("/tynd/kind"))
                .and_then(Value::as_str)
                .unwrap_or(if manifest.is_some() {
                    "community"
                } else {
                    "unresolved"
                });
            json!({ "pkg": package, "kind": kind, "family": family })
        })
        .collect::<Vec<_>>();
    if request.json {
        write_json(&json!({ "runtime": "bun", "plugins": entries }))?;
    } else if entries.is_empty() {
        step("(no plugins declared)");
    } else {
        for entry in entries {
            write_line(format!(
                "{:<7} {}",
                entry["kind"].as_str().unwrap_or("missing"),
                entry["pkg"].as_str().unwrap_or("unknown")
            ))?;
        }
    }
    Ok(())
}

/// `kind` column of `tynd plugin list`: `host` for a plugin backed by host APIs
/// (`tynd.kind: "host"`), `native` for one with a Rust crate, `js` otherwise.
fn plugin_list_kind(manifest: Option<&Value>) -> &'static str {
    match manifest {
        Some(value) if value.pointer("/tynd/kind").and_then(Value::as_str) == Some("host") => {
            "host"
        },
        Some(value) if native_section(value).is_some() => "native",
        Some(_) => "js",
        None => "unresolved",
    }
}

fn collect_plugin_checks(directory: &Path) -> Vec<PluginCheck> {
    let mut checks = Vec::new();
    let manifest_path = directory.join("package.json");
    let manifest = match read_json(&manifest_path) {
        Ok(value) => {
            checks.push(ok(
                "package.json",
                value["name"].as_str().unwrap_or("unnamed"),
            ));
            value
        },
        Err(error) => {
            checks.push(fail(
                "package.json",
                error.to_string(),
                "add a valid package.json",
            ));
            return checks;
        },
    };
    let name = manifest["name"].as_str().unwrap_or_default();
    check_package_metadata(&manifest, name, &mut checks);
    check_tynd_field(&manifest, &mut checks);
    check_package_exports(directory, &manifest, &mut checks);

    if let Some(native) = native_section(&manifest) {
        check_native_plugin(directory, name, native, &mut checks);
    }
    checks
}

/// Returns `tynd.native` when the manifest declares a native plugin; a missing,
/// `null` or `false` value means a JS-only plugin.
fn native_section(manifest: &Value) -> Option<&Value> {
    manifest
        .pointer("/tynd/native")
        .filter(|value| !value.is_null() && *value != &Value::Bool(false))
}

fn check_package_metadata(manifest: &Value, name: &str, checks: &mut Vec<PluginCheck>) {
    if RegexCache::package_name().is_match(name) {
        checks.push(ok("package name", name));
    } else {
        checks.push(fail(
            "package name",
            if name.is_empty() { "missing" } else { name },
            "use @scope/plugin-name or plugin-name",
        ));
    }
    match manifest.get("version").and_then(Value::as_str) {
        Some(version) if semver::Version::parse(version).is_ok() => {
            checks.push(ok("package version", version));
        },
        Some(version) => checks.push(fail(
            "package version",
            version,
            "set a valid semantic version",
        )),
        None => checks.push(fail("package version", "missing", "add version")),
    }
    match manifest.get("license").and_then(Value::as_str) {
        Some(license) if !license.trim().is_empty() => checks.push(ok("license", license)),
        _ => checks.push(fail("license", "missing", "add an SPDX license identifier")),
    }
    if manifest["type"] == "module" {
        checks.push(ok("package type", "module"));
    } else {
        checks.push(fail("package type", "not ESM", "set type to module"));
    }
    if manifest
        .pointer("/peerDependencies/@tynd~1core")
        .and_then(Value::as_str)
        .is_some()
    {
        checks.push(ok("@tynd/core peer", "declared"));
    } else {
        checks.push(fail(
            "@tynd/core peer",
            "missing",
            "declare peerDependencies.@tynd/core",
        ));
    }
}

/// Keys of the `package.json::tynd` block the CLI reads.
const TYND_KEYS: &[&str] = &["kind", "apis", "native", "backend"];
/// `tynd.kind` values of official plugins; community plugins omit the key.
const TYND_KINDS: &[&str] = &["host", "standalone"];
/// Keys of the `package.json::tynd.native` block the CLI reads.
const TYND_NATIVE_KEYS: &[&str] = &["path", "crateName", "features"];

fn check_tynd_field(manifest: &Value, checks: &mut Vec<PluginCheck>) {
    let Some(value) = manifest.get("tynd") else {
        return;
    };
    let Some(tynd) = value.as_object() else {
        checks.push(fail("tynd", "invalid", "set tynd to an object"));
        return;
    };
    let unknown = unknown_keys(tynd, TYND_KEYS);
    if unknown.is_empty() {
        checks.push(ok("tynd", "valid keys"));
    } else {
        checks.push(fail(
            "tynd",
            format!("unknown key(s) {}", unknown.join(", ")),
            format!("keep only {}", TYND_KEYS.join(", ")),
        ));
    }
    match tynd.get("kind") {
        None => {},
        Some(Value::String(kind)) if TYND_KINDS.contains(&kind.as_str()) => {
            checks.push(ok("tynd.kind", kind));
        },
        Some(kind) => checks.push(fail(
            "tynd.kind",
            kind.to_string(),
            "use \"host\" or \"standalone\", or omit kind for a community plugin",
        )),
    }
    if let Some(apis) = tynd.get("apis") {
        let invalid = apis.as_array().map_or_else(
            || vec![apis.to_string()],
            |apis| {
                apis.iter()
                    .filter(|api| {
                        !api.as_str()
                            .is_some_and(|api| crate::config::KNOWN_APIS.contains(&api))
                    })
                    .map(Value::to_string)
                    .collect()
            },
        );
        if invalid.is_empty() {
            checks.push(ok("tynd.apis", "valid"));
        } else {
            checks.push(fail(
                "tynd.apis",
                format!("invalid {}", invalid.join(", ")),
                "use an array of known host API names",
            ));
        }
    }
    match tynd.get("backend") {
        None | Some(Value::Bool(_)) => {},
        Some(backend) => checks.push(fail(
            "tynd.backend",
            backend.to_string(),
            "set tynd.backend to true or false",
        )),
    }
}

fn unknown_keys<'a>(object: &'a serde_json::Map<String, Value>, allowed: &[&str]) -> Vec<&'a str> {
    object
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect()
}

fn check_package_exports(directory: &Path, manifest: &Value, checks: &mut Vec<PluginCheck>) {
    let backend = manifest.pointer("/tynd/backend") == Some(&Value::Bool(true));
    let mut required = vec![".", "./client"];
    if backend {
        required.push("./backend");
    }
    match manifest.get("exports").and_then(Value::as_object) {
        Some(exports) => {
            for key in required {
                match exports.get(key) {
                    Some(value) => check_export_targets(directory, key, value, checks),
                    None => checks.push(fail(
                        format!("export {key}"),
                        "missing",
                        format!("declare package.json exports[{key:?}]"),
                    )),
                }
            }
        },
        None => checks.push(fail("exports", "missing", "declare . and ./client exports")),
    }
}

fn check_native_plugin(
    directory: &Path,
    package_name: &str,
    native_value: &Value,
    checks: &mut Vec<PluginCheck>,
) {
    let Some(native) = native_value.as_object() else {
        checks.push(fail(
            "tynd.native",
            "invalid",
            "set tynd.native to an object",
        ));
        return;
    };
    let unknown = unknown_keys(native, TYND_NATIVE_KEYS);
    if !unknown.is_empty() {
        checks.push(fail(
            "tynd.native",
            format!("unknown key(s) {}", unknown.join(", ")),
            format!("keep only {}", TYND_NATIVE_KEYS.join(", ")),
        ));
    }
    let raw_path = native
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("native");
    let native_dir = match checked_native_directory(directory, raw_path) {
        Ok(path) => {
            checks.push(ok("native path", raw_path));
            path
        },
        Err(error) => {
            checks.push(fail(
                "native path",
                error.to_string(),
                "keep native.path inside the package and add Cargo.toml",
            ));
            return;
        },
    };
    let crate_name = native
        .get("crateName")
        .and_then(Value::as_str)
        .map_or_else(|| native_crate_name(package_name), ToOwned::to_owned);
    match validate_native_crate_name(&crate_name) {
        Ok(()) => checks.push(ok("native crateName", &crate_name)),
        Err(error) => checks.push(fail(
            "native crateName",
            error.to_string(),
            "use a unique lowercase Rust identifier",
        )),
    }
    check_native_features(native, checks);
    check_native_cargo(&native_dir, checks);
    check_native_registration(&native_dir, checks);
}

fn check_native_features(native: &serde_json::Map<String, Value>, checks: &mut Vec<PluginCheck>) {
    let Some(features) = native.get("features") else {
        return;
    };
    let valid = features.as_array().is_some_and(|features| {
        features.iter().all(|feature| {
            feature.as_str().is_some_and(|feature| {
                !feature.is_empty()
                    && feature.chars().all(|character| {
                        character.is_ascii_alphanumeric() || "_-".contains(character)
                    })
            })
        })
    });
    if valid {
        checks.push(ok("native features", "valid"));
    } else {
        checks.push(fail(
            "native features",
            "invalid",
            "use an array of Cargo feature names",
        ));
    }
}

fn check_native_registration(native_dir: &Path, checks: &mut Vec<PluginCheck>) {
    match fs::read_to_string(native_dir.join("src/lib.rs")) {
        Ok(source)
            if (source.contains("#[plugin") && source.contains("#[plugin_impl]"))
                || (source.contains("register_plugin!") && source.contains("impl TyndPlugin")) =>
        {
            checks.push(ok("native registration", "declared"));
        },
        Ok(_) => checks.push(fail(
            "native registration",
            "missing",
            "use #[plugin] + #[plugin_impl] or register_plugin! + TyndPlugin",
        )),
        Err(error) => checks.push(fail(
            "native source",
            error.to_string(),
            "add native/src/lib.rs",
        )),
    }
    if native_dir.join("src/bin/gen-types.rs").is_file() {
        checks.push(ok("gen-types", "src/bin/gen-types.rs"));
    } else {
        checks.push(warning(
            "gen-types",
            "missing",
            "add native/src/bin/gen-types.rs",
        ));
    }
}

fn check_export_targets(directory: &Path, key: &str, value: &Value, checks: &mut Vec<PluginCheck>) {
    let mut targets = Vec::new();
    collect_export_targets(value, &mut targets);
    if targets.is_empty() {
        checks.push(fail(
            format!("export {key}"),
            "invalid",
            "export targets must be strings",
        ));
        return;
    }
    for target in targets {
        let relative = Path::new(target);
        let safe = target.starts_with("./")
            && relative.components().all(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::Normal(_)
                )
            });
        if !safe || !directory.join(relative).is_file() {
            checks.push(fail(
                format!("export {key}"),
                target,
                "point every export condition at an existing package file",
            ));
            return;
        }
    }
    checks.push(ok(format!("export {key}"), "valid"));
}

fn collect_export_targets<'a>(value: &'a Value, targets: &mut Vec<&'a str>) {
    match value {
        Value::String(target) => targets.push(target),
        Value::Object(conditions) => {
            for target in conditions.values() {
                collect_export_targets(target, targets);
            }
        },
        _ => {},
    }
}

fn checked_native_directory(directory: &Path, raw_path: &str) -> Result<PathBuf> {
    let relative = Path::new(raw_path);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("native.path escapes the plugin package");
    }
    let root = fs::canonicalize(directory)?;
    let native = fs::canonicalize(directory.join(relative))?;
    if !native.starts_with(root) || !native.join("Cargo.toml").is_file() {
        bail!("native directory or Cargo.toml is missing");
    }
    Ok(native)
}

fn check_native_cargo(native_dir: &Path, checks: &mut Vec<PluginCheck>) {
    let manifest_path = native_dir.join("Cargo.toml");
    let source = match fs::read_to_string(&manifest_path) {
        Ok(source) => source,
        Err(error) => {
            checks.push(fail(
                "native Cargo.toml",
                error.to_string(),
                "add native/Cargo.toml",
            ));
            return;
        },
    };
    let manifest = match toml::from_str::<toml::Value>(&source) {
        Ok(manifest) => manifest,
        Err(error) => {
            checks.push(fail(
                "native Cargo.toml",
                error.to_string(),
                "fix Cargo.toml syntax",
            ));
            return;
        },
    };
    let rlib = manifest
        .get("lib")
        .and_then(toml::Value::as_table)
        .and_then(|lib| lib.get("crate-type"))
        .and_then(toml::Value::as_array)
        .is_some_and(|types| types.iter().any(|value| value.as_str() == Some("rlib")));
    if rlib {
        checks.push(ok("native crate-type", "rlib"));
    } else {
        checks.push(fail(
            "native crate-type",
            "missing rlib",
            "set [lib] crate-type = [\"rlib\"]",
        ));
    }
    let sdk = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .is_some_and(|dependencies| dependencies.contains_key("tynd-plugin-sdk"));
    if sdk {
        checks.push(ok("native SDK", "tynd-plugin-sdk"));
    } else {
        checks.push(fail(
            "native SDK",
            "missing",
            "add tynd-plugin-sdk to [dependencies]",
        ));
    }
}

fn ok(name: impl Into<String>, detail: impl Into<String>) -> PluginCheck {
    PluginCheck {
        name: name.into(),
        status: "ok",
        detail: detail.into(),
        hint: None,
    }
}

fn warning(
    name: impl Into<String>,
    detail: impl Into<String>,
    hint: impl Into<String>,
) -> PluginCheck {
    PluginCheck {
        name: name.into(),
        status: "warn",
        detail: detail.into(),
        hint: Some(hint.into()),
    }
}

fn fail(
    name: impl Into<String>,
    detail: impl Into<String>,
    hint: impl Into<String>,
) -> PluginCheck {
    PluginCheck {
        name: name.into(),
        status: "fail",
        detail: detail.into(),
        hint: Some(hint.into()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{check_tynd_field, PluginCheck};

    fn tynd_checks(tynd: &Value) -> Vec<PluginCheck> {
        let mut checks = Vec::new();
        check_tynd_field(&json!({ "tynd": tynd }), &mut checks);
        checks
    }

    fn failures(tynd: &Value) -> Vec<String> {
        tynd_checks(tynd)
            .into_iter()
            .filter(|check| check.status == "fail")
            .map(|check| check.name)
            .collect()
    }

    #[test]
    fn tynd_field_accepts_the_documented_schema() {
        for tynd in [
            json!({ "kind": "host", "apis": ["fs"] }),
            json!({ "kind": "standalone", "backend": true, "apis": [] }),
            json!({ "native": { "path": "./native", "crateName": "acme-foo" } }),
            json!({ "backend": false }),
            json!({}),
        ] {
            assert!(failures(&tynd).is_empty(), "{tynd}");
        }
        let mut checks = Vec::new();
        check_tynd_field(&json!({ "name": "plugin-foo" }), &mut checks);
        assert!(checks.is_empty());
    }

    #[test]
    fn native_section_ignores_disabled_values() {
        for tynd in [
            json!({}),
            json!({ "native": null }),
            json!({ "native": false }),
        ] {
            assert!(
                super::native_section(&json!({ "tynd": tynd })).is_none(),
                "{tynd}"
            );
        }
        assert!(
            super::native_section(&json!({ "tynd": { "native": { "path": "./native" } } }))
                .is_some()
        );
    }

    #[test]
    fn plugin_list_labels_host_native_and_js_plugins() {
        let kind = |tynd: Value| super::plugin_list_kind(Some(&json!({ "tynd": tynd })));
        assert_eq!(
            kind(json!({ "kind": "host", "apis": ["clipboard"] })),
            "host"
        );
        assert_eq!(
            kind(json!({ "kind": "standalone", "native": { "path": "./native" } })),
            "native"
        );
        assert_eq!(kind(json!({ "kind": "standalone" })), "js");
        assert_eq!(kind(json!({})), "js");
        assert_eq!(super::plugin_list_kind(None), "unresolved");
    }

    #[test]
    fn tynd_field_rejects_unknown_keys_and_bad_values() {
        assert_eq!(failures(&json!("host")), ["tynd"]);
        assert_eq!(failures(&json!({ "sdk": true })), ["tynd"]);
        assert_eq!(failures(&json!({ "experimental": true })), ["tynd"]);
        assert_eq!(failures(&json!({ "kind": "external" })), ["tynd.kind"]);
        assert_eq!(failures(&json!({ "kind": 1 })), ["tynd.kind"]);
        assert_eq!(failures(&json!({ "apis": "fs" })), ["tynd.apis"]);
        assert_eq!(failures(&json!({ "apis": ["no-such-api"] })), ["tynd.apis"]);
        assert_eq!(failures(&json!({ "apis": [1] })), ["tynd.apis"]);
        assert_eq!(failures(&json!({ "backend": "yes" })), ["tynd.backend"]);
    }

    #[test]
    fn tynd_native_rejects_unknown_keys() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut checks = Vec::new();
        super::check_native_plugin(
            directory.path(),
            "plugin-foo",
            &json!({ "path": "./native", "crate_name": "foo" }),
            &mut checks,
        );
        assert!(checks
            .iter()
            .any(|check| check.name == "tynd.native" && check.status == "fail"));
    }
}
