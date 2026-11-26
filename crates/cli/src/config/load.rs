use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::fsx::absolute;

use super::{
    environment::{
        config_override_paths, deep_merge, load_dotenv_files, load_script_config, strip_jsonc,
    },
    extends::load_config_tree,
    model::{ConfigError, LoadedConfig, ResolvedConfig, TyndConfig, CONFIG_FILES},
    shared::resolve_path,
    validate::validate_config_at,
};

pub(crate) fn load(cwd: &Path, env: Option<&str>) -> Result<ResolvedConfig> {
    let loaded = load_typed(cwd, env)?;
    Ok(ResolvedConfig {
        value: serde_json::to_value(loaded.config)?,
        path: loaded.path,
    })
}

pub(crate) fn load_typed(cwd: &Path, env: Option<&str>) -> Result<LoadedConfig> {
    let selected_env = env.map(str::to_owned).or_else(|| {
        std::env::var("TYND_ENV")
            .ok()
            .filter(|value| !value.trim().is_empty())
    });
    let env = selected_env.as_deref();
    let cwd = absolute(cwd)?;
    let path = find_config_path(&cwd);
    let root = path
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(&cwd)
        .to_path_buf();

    let dotenv = load_dotenv_files(&root, env)?;
    let mut stack = HashSet::new();
    let mut sources = Vec::new();
    let mut value = match &path {
        Some(path) => load_config_tree(path, env, &dotenv, &mut stack, &mut sources)?,
        None => Value::Object(Map::new()),
    };

    if let Some(base_path) = path.as_deref() {
        for override_path in config_override_paths(base_path, env) {
            let override_value =
                load_config_tree(&override_path, env, &dotenv, &mut stack, &mut sources)?;
            deep_merge(&mut value, override_value);
        }
    }

    let mut with_defaults = json!({
        "backend": "backend/main.ts",
        "frontendDir": "frontend"
    });
    deep_merge(&mut with_defaults, value);
    let mut config: TyndConfig = serde_json::from_value(with_defaults).map_err(|error| {
        ConfigError::new(
            path.as_deref(),
            vec![format!("configuration shape: {error}")],
        )
    })?;
    validate_config_at(&config, path.as_deref())?;
    resolve_paths(&mut config, &root);
    let environment = dotenv
        .into_iter()
        .filter(|(key, _)| std::env::var_os(key).is_none())
        .collect();
    Ok(LoadedConfig {
        config,
        path,
        root,
        sources,
        environment,
    })
}

pub(super) fn load_raw_config_with_env(
    path: &Path,
    env: Option<&str>,
    dotenv: &BTreeMap<String, String>,
) -> Result<Value> {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "ts" | "mts" | "cts" | "js" | "mjs" | "cjs" => load_script_config(path, env, dotenv),
        "json" => {
            let source =
                fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str(&source)
                .with_context(|| format!("parse JSON config {}", path.display()))
        },
        "jsonc" => {
            let source =
                fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str(&strip_jsonc(&source))
                .with_context(|| format!("parse JSONC config {}", path.display()))
        },
        "toml" => {
            let source =
                fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            let value: toml::Value = toml::from_str(&source)
                .with_context(|| format!("parse TOML config {}", path.display()))?;
            serde_json::to_value(value).context("convert TOML config")
        },
        "yaml" | "yml" => {
            let source =
                fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
            serde_yaml::from_str(&source)
                .with_context(|| format!("parse YAML config {}", path.display()))
        },
        _ => bail!("unsupported config format: {}", path.display()),
    }
}

pub(crate) fn find_config_in_directory(directory: &Path) -> Option<PathBuf> {
    let mut directory = absolute(directory).ok()?;
    if directory.is_file() {
        directory.pop();
    }
    CONFIG_FILES.iter().find_map(|name| {
        let candidate = directory.join(name);
        candidate.is_file().then_some(candidate)
    })
}

pub(crate) fn find_config_path(cwd: &Path) -> Option<PathBuf> {
    let mut directory = absolute(cwd).ok()?;
    if directory.is_file() {
        directory.pop();
    }
    loop {
        if let Some(path) = find_config_in_directory(&directory) {
            return Some(path);
        }
        if !directory.pop() {
            return None;
        }
    }
}

pub(crate) fn find_project_root(cwd: &Path) -> Result<PathBuf> {
    let cwd = absolute(cwd)?;
    Ok(find_config_path(&cwd)
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or(cwd))
}

pub(crate) fn resolve_paths(config: &mut TyndConfig, root: &Path) {
    config.backend = resolve_path(root, &config.backend);
    config.frontend_dir = resolve_path(root, &config.frontend_dir);
    for sidecar in &mut config.sidecars {
        sidecar.path = resolve_path(root, &sidecar.path);
    }
    if let Some(signing) = config
        .bundle
        .as_mut()
        .and_then(|bundle| bundle.sign.as_mut())
    {
        if let Some(windows) = &mut signing.windows {
            if !windows.certificate.starts_with("cert:")
                && !looks_like_sha1_thumbprint(&windows.certificate)
            {
                windows.certificate = resolve_path(root, Path::new(&windows.certificate))
                    .to_string_lossy()
                    .into();
            }
        }
        if let Some(macos) = &mut signing.macos {
            macos.identity = resolve_path(root, &macos.identity);
            if let Some(entitlements) = &mut macos.entitlements {
                *entitlements = resolve_path(root, entitlements);
            }
            if let Some(notarize) = &mut macos.notarize {
                if !notarize.api_key.starts_with("env:") {
                    notarize.api_key = resolve_path(root, Path::new(&notarize.api_key))
                        .to_string_lossy()
                        .into();
                }
            }
        }
    }
}

fn looks_like_sha1_thumbprint(value: &str) -> bool {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    compact.len() == 40
        && compact
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}
