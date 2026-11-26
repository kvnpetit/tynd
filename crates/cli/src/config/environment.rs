use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{ensure, Context, Result};
use serde_json::Value;

pub(super) fn config_override_paths(base: &Path, env: Option<&str>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(environment) = env.filter(|value| !value.is_empty()) {
        if let Some(path) = suffixed_config_path(base, &[environment]) {
            paths.push(path);
        }
    }
    if let Some(path) = suffixed_config_path(base, &["local"]) {
        paths.push(path);
    }
    if let Some(environment) = env.filter(|value| !value.is_empty()) {
        if let Some(path) = suffixed_config_path(base, &[environment, "local"]) {
            paths.push(path);
        }
    }
    paths
}

fn suffixed_config_path(base: &Path, suffixes: &[&str]) -> Option<PathBuf> {
    if suffixes.iter().any(|suffix| {
        suffix.is_empty()
            || !suffix
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
    }) {
        return None;
    }
    let parent = base.parent()?;
    let stem = base.file_stem()?.to_str()?;
    let suffix = suffixes.join(".");
    super::model::CONFIG_EXTENSIONS
        .iter()
        .find_map(|extension| {
            let candidate = parent.join(format!("{stem}.{suffix}.{extension}"));
            candidate.is_file().then_some(candidate)
        })
}

pub(super) fn load_dotenv_files(
    root: &Path,
    env: Option<&str>,
) -> Result<BTreeMap<String, String>> {
    let mut names = vec![".env".to_owned(), ".env.local".to_owned()];
    if let Some(environment) = env.filter(|value| !value.is_empty()) {
        ensure!(
            environment
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character)),
            "environment name contains unsupported characters"
        );
        names.push(format!(".env.{environment}"));
        names.push(format!(".env.{environment}.local"));
    }
    let mut values = BTreeMap::new();
    for name in names {
        let path = root.join(name);
        if !path.is_file() {
            continue;
        }
        let metadata = fs::metadata(&path)?;
        ensure!(
            metadata.len() <= 1024 * 1024,
            "dotenv file exceeds 1 MiB: {}",
            path.display()
        );
        let source =
            fs::read_to_string(&path).with_context(|| format!("read dotenv {}", path.display()))?;
        for (line_index, line) in source.lines().enumerate() {
            if let Some((key, value)) = parse_dotenv_line(line)
                .with_context(|| format!("{}:{}", path.display(), line_index + 1))?
            {
                values.insert(key, value);
            }
        }
    }
    Ok(values)
}

fn parse_dotenv_line(line: &str) -> Result<Option<(String, String)>> {
    let mut line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    if let Some(rest) = line.strip_prefix("export ") {
        line = rest.trim_start();
    }
    let (key, raw_value) = line
        .split_once('=')
        .context("dotenv line must contain '='")?;
    let key = key.trim();
    ensure!(
        key.chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
            && key
                .chars()
                .all(|character| character == '_' || character.is_ascii_alphanumeric()),
        "invalid dotenv variable name"
    );
    let raw_value = raw_value.trim();
    let value = if raw_value.starts_with('"') {
        ensure!(
            raw_value.len() >= 2 && raw_value.ends_with('"'),
            "unterminated dotenv string"
        );
        unescape_dotenv_double(&raw_value[1..raw_value.len() - 1])?
    } else if raw_value.starts_with('\'') {
        ensure!(
            raw_value.len() >= 2 && raw_value.ends_with('\''),
            "unterminated dotenv string"
        );
        raw_value[1..raw_value.len() - 1].to_owned()
    } else {
        strip_dotenv_comment(raw_value).trim_end().to_owned()
    };
    Ok(Some((key.to_owned(), value)))
}

fn strip_dotenv_comment(value: &str) -> &str {
    for (index, character) in value.char_indices() {
        if character == '#'
            && value[..index]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
        {
            return &value[..index];
        }
    }
    value
}

fn unescape_dotenv_double(value: &str) -> Result<String> {
    let mut output = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let escaped = characters.next().context("dangling dotenv escape")?;
        output.push(match escaped {
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            '\\' => '\\',
            '"' => '"',
            other => other,
        });
    }
    Ok(output)
}

pub(super) fn load_script_config(
    path: &Path,
    env: Option<&str>,
    dotenv: &BTreeMap<String, String>,
) -> Result<Value> {
    const MARKER: &str = "__TYND_CONFIG_VALUE__";
    const BRIDGE: &str = r#"
import { pathToFileURL } from "node:url";
const file = process.env.TYND_CONFIG_BRIDGE_PATH;
if (!file) throw new Error("missing TYND_CONFIG_BRIDGE_PATH");
const module = await import(pathToFileURL(file).href + "?tynd=" + Date.now());
let value = module.default ?? module;
if (typeof value === "function") {
  value = await value({ envName: process.env.TYND_CONFIG_ENV ?? "development" });
}
process.stdout.write("\n__TYND_CONFIG_VALUE__" + JSON.stringify(value ?? {}));
"#;

    let bridge_path = dunce::simplified(path);
    let mut command = crate::process::CommandSpec::new("bun")
        .args(["--eval", BRIDGE])
        .env("TYND_CONFIG_BRIDGE_PATH", bridge_path.as_os_str())
        .env("TYND_CONFIG_ENV", env.unwrap_or("development"))
        .timeout(std::time::Duration::from_secs(20))
        .max_output_bytes(4 * 1024 * 1024);
    if let Some(parent) = path.parent() {
        command = command.cwd(parent);
    }
    for (key, value) in dotenv {
        if std::env::var_os(key).is_none() && !crate::process::is_sensitive_env_key(key) {
            command = command.env(key, value);
        }
    }
    let output = crate::process::run_spec(command)
        .with_context(|| format!("load script config {}", path.display()))?;
    let (_, encoded) = output
        .stdout
        .rsplit_once(MARKER)
        .with_context(|| format!("Bun did not return a config value for {}", path.display()))?;
    serde_json::from_str(encoded.trim())
        .with_context(|| format!("parse Bun config output for {}", path.display()))
}

pub(super) fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base.insert(key, value);
                    },
                }
            }
        },
        (base, overlay) => *base = overlay,
    }
}

pub(super) fn strip_jsonc(source: &str) -> String {
    let mut without_comments = String::with_capacity(source.len());
    let mut characters = source.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(character) = characters.next() {
        if in_string {
            without_comments.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            without_comments.push(character);
            continue;
        }
        if character == '/' && characters.peek() == Some(&'/') {
            characters.next();
            for next in characters.by_ref() {
                if next == '\n' {
                    without_comments.push('\n');
                    break;
                }
            }
            continue;
        }
        if character == '/' && characters.peek() == Some(&'*') {
            characters.next();
            let mut previous = '\0';
            for next in characters.by_ref() {
                if next == '\n' {
                    without_comments.push('\n');
                }
                if previous == '*' && next == '/' {
                    break;
                }
                previous = next;
            }
            continue;
        }
        without_comments.push(character);
    }

    let mut output = String::with_capacity(without_comments.len());
    let mut iterator = without_comments.chars().peekable();
    in_string = false;
    escaped = false;
    while let Some(character) = iterator.next() {
        if in_string {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        if character == '"' {
            in_string = true;
            output.push(character);
            continue;
        }
        if character == ',' {
            let mut lookahead = iterator.clone();
            while lookahead.peek().is_some_and(|next| next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }
        output.push(character);
    }
    output
}
