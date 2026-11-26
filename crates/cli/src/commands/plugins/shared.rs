use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::fsx::read_json;

pub(super) struct ResolvedPluginInput {
    pub(super) name: String,
    pub(super) root: PathBuf,
}

pub(super) fn resolve_plugin_input(cwd: &Path, input: &str) -> Result<ResolvedPluginInput> {
    let input_path = PathBuf::from(input);
    let direct = if input_path.is_absolute() {
        input_path
    } else {
        cwd.join(&input_path)
    };
    let candidates = [
        direct,
        cwd.join("plugins").join(input),
        cwd.join("node_modules").join(input),
    ];
    for candidate in candidates {
        let manifest_path = candidate.join("package.json");
        if !manifest_path.is_file() {
            continue;
        }
        let manifest = read_json(&manifest_path)?;
        let name = manifest
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .with_context(|| format!("{} has no package name", manifest_path.display()))?;
        return Ok(ResolvedPluginInput {
            name: name.to_owned(),
            root: fs::canonicalize(&candidate)
                .with_context(|| format!("resolve plugin path {}", candidate.display()))?,
        });
    }
    let cwd_manifest = cwd.join("package.json");
    if cwd_manifest.is_file() {
        let manifest = read_json(&cwd_manifest)?;
        if manifest.get("name").and_then(Value::as_str) == Some(input) {
            return Ok(ResolvedPluginInput {
                name: input.to_owned(),
                root: fs::canonicalize(cwd)
                    .with_context(|| format!("resolve plugin path {}", cwd.display()))?,
            });
        }
    }
    bail!("plugin {input:?} not found; pass a plugin directory or an installed package name")
}

pub(super) fn native_crate_name(package_name: &str) -> String {
    package_name
        .trim_start_matches('@')
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Rust identifier of the Cargo package `crate_name`: Cargo maps `-` to `_`.
pub(super) fn native_crate_ident(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

pub(super) fn validate_native_crate_name(value: &str) -> Result<()> {
    const KEYWORDS: &[&str] = &[
        "as", "async", "await", "become", "box", "break", "const", "continue", "crate", "do",
        "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl", "in",
        "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
        "typeof", "union", "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
    ];
    if KEYWORDS.contains(&value)
        || !value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_lowercase())
        || !value.chars().all(|character| {
            matches!(character, '_' | '-')
                || character.is_ascii_lowercase()
                || character.is_ascii_digit()
        })
    {
        bail!("native crateName must be a non-keyword lowercase Cargo package name, got {value:?}");
    }
    Ok(())
}

pub(super) struct PluginIdentity<'a> {
    pub(super) scope: Option<&'a str>,
    pub(super) local_name: &'a str,
}

pub(super) fn parse_plugin_name(name: &str) -> Result<PluginIdentity<'_>> {
    let (scope, local_name) = if let Some(scoped) = name.strip_prefix('@') {
        let (scope, local_name) = scoped
            .split_once('/')
            .context("scoped plugin name must use @scope/name")?;
        if !RegexCache::simple_name().is_match(scope) {
            bail!("plugin scope must contain only lowercase letters, digits, dots, underscores, and hyphens");
        }
        (Some(scope), local_name)
    } else {
        (None, name)
    };
    if !RegexCache::simple_name().is_match(local_name) {
        bail!("plugin name must contain only lowercase letters, digits, dots, underscores, and hyphens");
    }
    Ok(PluginIdentity { scope, local_name })
}

pub(super) struct RegexCache;
impl RegexCache {
    pub(super) fn simple_name() -> regex::Regex {
        regex::Regex::new(r"^[a-z0-9][a-z0-9._-]*$").expect("static regex")
    }
    pub(super) fn package_name() -> regex::Regex {
        regex::Regex::new(r"^(@[a-z0-9][a-z0-9._-]*/)?plugin-[a-z0-9][a-z0-9._-]*$")
            .expect("static regex")
    }
}

pub(super) fn resolve_plugin_manifest(cwd: &Path, package: &str) -> Option<Value> {
    let direct = cwd.join(package).join("package.json");
    let node = cwd.join("node_modules").join(package).join("package.json");
    [direct, node]
        .into_iter()
        .find(|path| path.is_file())
        .and_then(|path| read_json(&path).ok())
}

#[cfg(test)]
mod tests {
    use super::{native_crate_ident, native_crate_name, validate_native_crate_name};

    #[test]
    fn native_crate_names_are_hyphenated_cargo_packages() {
        assert_eq!(
            native_crate_name("@acme/tynd-plugin-foo"),
            "acme-tynd-plugin-foo"
        );
        assert_eq!(native_crate_name("My.Plugin"), "my-plugin");
        assert_eq!(
            native_crate_ident("tynd-plugin-demo-capture"),
            "tynd_plugin_demo_capture"
        );
    }

    #[test]
    fn native_crate_name_validation_accepts_hyphens_and_rejects_keywords() {
        for valid in ["tynd-plugin-audio", "tynd_plugin_audio", "a2"] {
            assert!(validate_native_crate_name(valid).is_ok(), "{valid}");
        }
        for invalid in ["", "-lead", "2fast", "Upper", "self", "with space", "a.b"] {
            assert!(validate_native_crate_name(invalid).is_err(), "{invalid}");
        }
    }
}
