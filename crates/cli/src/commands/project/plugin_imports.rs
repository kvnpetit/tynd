//! Check on plugin-first API imports: a value import of a plugin-first API
//! (`@tynd/plugin-<api>` or `@tynd/plugin-<api>/client`) while the project
//! does not declare the matching plugin in `tynd.config`. Without the
//! declaration the host is compiled without that API and the call fails at runtime with
//! `api_disabled`, so `tynd build` refuses it while `tynd validate` and
//! `tynd dev` warn. Type-only imports (`import type`, `export type`,
//! `import { type A }`) and specifiers outside module loads (comments,
//! `vi.mock("...")`, config arrays) are fine.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
use regex::Regex;
use walkdir::WalkDir;

use crate::{
    config::{self, TyndConfig, API_PLUGIN_PACKAGES},
    output::warn,
};

const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const SKIPPED_DIRS: &[&str] = &["node_modules", "target", "dist", "build", "out", "coverage"];
pub(super) const MAX_SOURCE_BYTES: u64 = 1024 * 1024;
const MAX_LISTED_FILES: usize = 3;

pub(super) fn collect_undeclared_plugin_imports(
    loaded: &config::LoadedConfig,
    warnings: &mut Vec<String>,
) {
    warnings.extend(plugin_import_warnings(&loaded.config, &loaded.root));
}

/// `tynd build`: fail instead of shipping a host without the imported plugin.
pub(super) fn ensure_plugin_imports_declared(loaded: &config::LoadedConfig) -> Result<()> {
    let issues = plugin_import_warnings(&loaded.config, &loaded.root);
    if !issues.is_empty() {
        bail!(
            "{}",
            issues.join(
                "
"
            )
        );
    }
    Ok(())
}

/// `tynd dev`: report undeclared plugin imports at startup.
pub(super) fn warn_undeclared_plugin_imports(loaded: &config::LoadedConfig) {
    for issue in plugin_import_warnings(&loaded.config, &loaded.root) {
        warn(issue);
    }
}

/// Files (relative to the project root) importing an undeclared plugin API.
type ImportScan = BTreeMap<&'static str, Vec<PathBuf>>;

fn plugin_import_warnings(config: &TyndConfig, root: &Path) -> Vec<String> {
    let scan = scan_plugin_imports(config, root);
    scan.into_iter()
        .map(|(api, files)| {
            let package = package_for_api(api);
            format!(
                "{} imports the \"{api}\" API but plugin \"{package}\" is not declared in \
                 tynd.config plugins; the call fails at runtime with api_disabled \
                 (run `tynd add {api}` to declare it)",
                list_files(&files)
            )
        })
        .collect()
}

/// Walks the project's source files (skipping dependency/build directories
/// and files over `MAX_SOURCE_BYTES`) and records plugin-first imports.
fn scan_plugin_imports(config: &TyndConfig, root: &Path) -> ImportScan {
    let import = specifier_pattern();
    let mut scan = ImportScan::default();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| !is_skipped(entry.path(), root))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let path = entry.path();
        if !has_source_extension(path)
            || entry
                .metadata()
                .map_or(true, |meta| meta.len() > MAX_SOURCE_BYTES)
        {
            continue;
        }
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        let relative = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        for specifier in value_import_specifiers(&source, &import) {
            let files = match classify_specifier(specifier) {
                Some(api) if !declares_api(config, api) => scan.entry(api).or_default(),
                _ => continue,
            };
            if !files.contains(&relative) {
                files.push(relative.clone());
            }
        }
    }
    scan
}

fn list_files(files: &[PathBuf]) -> String {
    let mut listed: Vec<String> = files
        .iter()
        .take(MAX_LISTED_FILES)
        .map(|file| file.to_string_lossy().replace('\\', "/"))
        .collect();
    if files.len() > MAX_LISTED_FILES {
        listed.push(format!("{} more", files.len() - MAX_LISTED_FILES));
    }
    listed.join(", ")
}

/// A quoted plugin specifier; group 1 is the specifier. Matches are then
/// checked against the comment- and string-blanked code shape.
fn specifier_pattern() -> Regex {
    Regex::new(r#"["'](@tynd/plugin-[a-z][a-z0-9-]*(?:/client)?)["']"#)
        .expect("static specifier pattern is valid")
}

/// Plugin specifiers `source` loads as values: `import ... from`,
/// `export ... from`, bare `import "x"`, `import("x")` and `require("x")`.
/// Specifiers in comments, templates or other strings, plain string arguments
/// (`vi.mock("x")`), arrays and type-only declarations are skipped.
fn value_import_specifiers<'a>(source: &'a str, pattern: &Regex) -> Vec<&'a str> {
    // Comments and string contents blanked: code structure only.
    let code = crate::source_edit::code_shape(source, false);
    // Same, but words inside `'`/`"` strings kept: a specifier survives here
    // only when it is a real string literal, not comment or template text.
    let literals = crate::source_edit::code_shape(source, true);
    let mut specifiers = Vec::new();
    for capture in pattern.captures_iter(source) {
        let (Some(quoted), Some(specifier)) = (capture.get(0), capture.get(1)) else {
            continue;
        };
        if literals.get(specifier.range()) != Some(specifier.as_str()) {
            continue;
        }
        if code.is_char_boundary(quoted.start()) && is_value_import(&code, quoted.start()) {
            specifiers.push(specifier.as_str());
        }
    }
    specifiers
}

/// Whether the string literal opening at `quote` is the module specifier of a
/// value import, judged from the code preceding it.
fn is_value_import(code: &str, quote: usize) -> bool {
    let before = code[..quote].trim_end();
    if let Some(rest) = before.strip_suffix('(') {
        // `import("x")` / `require("x")`; any other call is not an import.
        return matches!(
            trailing_identifier(rest.trim_end()),
            Some("import" | "require")
        );
    }
    match trailing_identifier(before) {
        Some("import") => true,
        Some("from") => {
            let head = &before[..before.len() - "from".len()];
            statement_clause(head).is_some_and(|clause| !is_type_only_clause(clause))
        },
        _ => false,
    }
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

/// Start of the identifier ending `code` (`code.len()` when there is none).
fn identifier_start(code: &str) -> usize {
    code.rfind(|ch: char| !is_identifier_char(ch))
        .map_or(0, |index| index + 1)
}

/// The identifier ending `code`, unless it is a property access (`a.import`).
fn trailing_identifier(code: &str) -> Option<&str> {
    let start = identifier_start(code);
    let word = &code[start..];
    let property = code[..start].trim_end().ends_with('.');
    (!word.is_empty() && !property).then_some(word)
}

/// The clause between the `import`/`export` keyword opening the current
/// statement and its `from`. Only the nearest keyword counts, so a preceding
/// statement without a semicolon (`export type A = B`) is never merged in.
fn statement_clause(head: &str) -> Option<&str> {
    let mut end = head.len();
    loop {
        let candidate = &head[..end];
        let word_end = candidate.rfind(is_identifier_char)? + 1;
        let word_start = identifier_start(&candidate[..word_end]);
        let word = &candidate[word_start..word_end];
        if matches!(word, "import" | "export") && !candidate[..word_start].trim_end().ends_with('.')
        {
            return Some(head[word_end..].trim());
        }
        end = word_start;
    }
}

/// `type X`, `type { .. }`, `type * ..`, or braces whose members are all
/// `type`-prefixed (`{ type A, type B as C }`).
fn is_type_only_clause(clause: &str) -> bool {
    if let Some(rest) = clause.strip_prefix("type") {
        // `import type from "x"` imports a default binding named `type`.
        if rest.starts_with(|ch: char| ch.is_whitespace() || ch == '{' || ch == '*') {
            return true;
        }
    }
    let Some(members) = clause
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
    else {
        return false;
    };
    let mut members = members
        .split(',')
        .map(str::trim)
        .filter(|member| !member.is_empty())
        .peekable();
    members.peek().is_some()
        && members.all(|member| {
            let words: Vec<&str> = member.split_whitespace().collect();
            // `type as t` renames a value binding named `type`.
            words.len() >= 2 && words[0] == "type" && !(words.len() == 3 && words[1] == "as")
        })
}

pub(super) fn is_skipped(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    path.is_dir() && (name.starts_with('.') || SKIPPED_DIRS.contains(&name))
}

pub(super) fn has_source_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| SOURCE_EXTENSIONS.contains(&extension))
}

/// Map an import specifier to the plugin-first API it imports.
fn classify_specifier(specifier: &str) -> Option<&'static str> {
    let package = specifier.strip_suffix("/client").unwrap_or(specifier);
    API_PLUGIN_PACKAGES
        .iter()
        .find(|(_, candidate)| *candidate == package)
        .map(|(api, _)| *api)
}

fn package_for_api(api: &str) -> &'static str {
    API_PLUGIN_PACKAGES
        .iter()
        .find(|(candidate, _)| *candidate == api)
        .map_or("", |(_, package)| *package)
}

/// A plugin-first API is declared when its plugin package is listed in
/// `plugins`, or when the API is enabled in the top-level `apis`.
fn declares_api(config: &TyndConfig, api: &str) -> bool {
    let package = package_for_api(api);
    config.apis.iter().any(|enabled| enabled == api)
        || config
            .plugins
            .iter()
            .any(|plugin| plugin.package() == package)
}

#[cfg(test)]
mod tests;
