use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, ensure, Context, Result};
use serde_json::{Map, Value};

use super::{environment::deep_merge, load::load_raw_config_with_env, model::CONFIG_FILES};

pub(super) fn load_config_tree(
    path: &Path,
    env: Option<&str>,
    dotenv: &BTreeMap<String, String>,
    stack: &mut HashSet<PathBuf>,
    sources: &mut Vec<PathBuf>,
) -> Result<Value> {
    let canonical =
        fs::canonicalize(path).with_context(|| format!("resolve config {}", path.display()))?;
    if !stack.insert(canonical.clone()) {
        bail!("cyclic config extends detected at {}", canonical.display());
    }
    let result = (|| {
        let mut current = load_raw_config_with_env(&canonical, env, dotenv)?;
        take_schema(&mut current, &canonical)?;
        let extends = take_extends(&mut current, &canonical)?;
        let mut combined = Value::Object(Map::new());
        for specifier in extends {
            let inherited_path = resolve_extends_path(&canonical, &specifier)?;
            let inherited = load_config_tree(&inherited_path, env, dotenv, stack, sources)?;
            deep_merge(&mut combined, inherited);
        }
        if !sources.contains(&canonical) {
            sources.push(canonical.clone());
        }
        deep_merge(&mut combined, current);
        Ok(combined)
    })();
    stack.remove(&canonical);
    result
}

/// Drops the editor-only top-level `$schema` key of one config file.
///
/// Only a string is accepted, so a typo such as `"$schema": true` still fails
/// loudly. A nested `$schema` is left in place and rejected by the typed model.
fn take_schema(value: &mut Value, path: &Path) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        bail!("config {} must export an object", path.display());
    };
    match object.remove("$schema") {
        None | Some(Value::String(_)) => Ok(()),
        Some(_) => bail!("config $schema in {} must be a string", path.display()),
    }
}

fn take_extends(value: &mut Value, path: &Path) -> Result<Vec<String>> {
    let Some(object) = value.as_object_mut() else {
        bail!("config {} must export an object", path.display());
    };
    let Some(extends) = object.remove("extends") else {
        return Ok(Vec::new());
    };
    match extends {
        Value::String(value) if !value.trim().is_empty() => Ok(vec![value]),
        Value::Array(values) => values
            .into_iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_owned)
                    .with_context(|| {
                        format!(
                            "config extends entries in {} must be strings",
                            path.display()
                        )
                    })
            })
            .collect(),
        _ => bail!(
            "config extends in {} must be a string or string array",
            path.display()
        ),
    }
}

fn resolve_extends_path(base_config: &Path, specifier: &str) -> Result<PathBuf> {
    let specifier = specifier.trim();
    ensure_local_extends_specifier(specifier)?;
    let parent = base_config.parent().context("config path has no parent")?;
    let candidate = Path::new(specifier);
    if candidate.is_absolute()
        || specifier.starts_with('.')
        || specifier.contains('\\')
        || specifier.starts_with('/')
    {
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            parent.join(candidate)
        };
        return resolve_config_candidate(&candidate)
            .with_context(|| format!("resolve config extends {specifier:?}"));
    }
    resolve_npm_config(parent, specifier)
        .with_context(|| format!("resolve npm config extends {specifier:?}"))
}

fn ensure_local_extends_specifier(specifier: &str) -> Result<()> {
    ensure!(!specifier.is_empty(), "config extends must not be empty");
    let lower = specifier.to_ascii_lowercase();
    ensure!(
        !lower.starts_with("http://")
            && !lower.starts_with("https://")
            && !lower.starts_with("file://")
            && !lower.contains("://"),
        "remote config extends are not supported: {specifier}"
    );
    Ok(())
}

fn resolve_npm_config(start: &Path, specifier: &str) -> Result<PathBuf> {
    let (package, subpath) = split_package_specifier(specifier)?;
    let mut directory = Some(start.to_path_buf());
    while let Some(current) = directory {
        let package_dir = current.join("node_modules").join(&package);
        if package_dir.join("package.json").is_file() {
            let package_real = fs::canonicalize(&package_dir)?;
            let target = if let Some(subpath) = &subpath {
                resolve_config_candidate(&package_dir.join(subpath))?
            } else {
                resolve_package_config_entry(&package_dir)?
            };
            let target_real = fs::canonicalize(&target)?;
            ensure!(
                target_real.starts_with(&package_real),
                "npm config extends escapes package {package}"
            );
            return Ok(target_real);
        }
        directory = current.parent().map(Path::to_path_buf);
    }
    bail!("npm config package {package:?} is not installed")
}

fn split_package_specifier(specifier: &str) -> Result<(String, Option<PathBuf>)> {
    let parts = specifier.split('/').collect::<Vec<_>>();
    let package_parts = if specifier.starts_with('@') { 2 } else { 1 };
    ensure!(
        parts.len() >= package_parts && parts[..package_parts].iter().all(|part| !part.is_empty()),
        "invalid npm config specifier {specifier:?}"
    );
    let package = parts[..package_parts].join("/");
    let subpath = if parts.len() > package_parts {
        let path = PathBuf::from(parts[package_parts..].join("/"));
        ensure!(
            path.components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
            "npm config subpath must not contain dot or parent segments"
        );
        Some(path)
    } else {
        None
    };
    Ok((package, subpath))
}

fn resolve_package_config_entry(package_dir: &Path) -> Result<PathBuf> {
    let manifest_path = package_dir.join("package.json");
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path)?)
        .with_context(|| format!("parse {}", manifest_path.display()))?;
    for key in ["tyndConfig", "tynd"] {
        if let Some(entry) = manifest.get(key).and_then(Value::as_str) {
            return resolve_config_candidate(&package_dir.join(entry));
        }
    }
    if let Some(entry) = manifest
        .pointer("/exports/~1tynd-config")
        .and_then(resolve_export_target)
    {
        return resolve_config_candidate(&package_dir.join(entry));
    }
    for name in CONFIG_FILES {
        let candidate = package_dir.join(name);
        if candidate.is_file() {
            return fs::canonicalize(candidate).map_err(Into::into);
        }
    }
    for name in [
        "index.ts",
        "index.mts",
        "index.js",
        "index.mjs",
        "index.json",
    ] {
        let candidate = package_dir.join(name);
        if candidate.is_file() {
            return fs::canonicalize(candidate).map_err(Into::into);
        }
    }
    bail!("package {} has no Tynd config entry", package_dir.display())
}

fn resolve_export_target(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value),
        Value::Object(object) => ["import", "default", "require"]
            .into_iter()
            .find_map(|key| object.get(key).and_then(resolve_export_target)),
        _ => None,
    }
}

fn resolve_config_candidate(candidate: &Path) -> Result<PathBuf> {
    if candidate.is_file() {
        return fs::canonicalize(candidate).map_err(Into::into);
    }
    if candidate.is_dir() {
        for name in CONFIG_FILES {
            let nested = candidate.join(name);
            if nested.is_file() {
                return fs::canonicalize(nested).map_err(Into::into);
            }
        }
    }
    if candidate.extension().is_none() {
        for extension in [
            "ts", "mts", "cts", "js", "mjs", "cjs", "json", "jsonc", "toml", "yaml", "yml",
        ] {
            let nested = candidate.with_extension(extension);
            if nested.is_file() {
                return fs::canonicalize(nested).map_err(Into::into);
            }
        }
    }
    bail!("config file not found: {}", candidate.display())
}

#[cfg(test)]
mod tests;
