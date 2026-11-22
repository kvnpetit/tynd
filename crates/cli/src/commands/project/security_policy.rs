//! `tynd validate` / `tynd build` check: the app calls scope-gated host APIs
//! (fs, http, process, shell) or loads plugins while no security policy is
//! configured. Resource scopes are open by default; a policy set from the
//! backend with `app.start({ security: { ... } })` narrows them.

use std::{fs, path::Path};

use regex::Regex;
use walkdir::WalkDir;

use super::plugin_imports::{has_source_extension, is_skipped, MAX_SOURCE_BYTES};
use crate::config::{self, TyndConfig, API_PLUGIN_PACKAGES};

/// Host APIs whose resource scopes the security policy narrows.
const SCOPED_APIS: &[&str] = &["fs", "http", "process", "shell"];

/// Add the open-scope warning to `warnings` when it applies.
pub(super) fn collect_open_scope_warning(
    loaded: &config::LoadedConfig,
    warnings: &mut Vec<String>,
) {
    warnings.extend(open_scope_warning(&loaded.config, &loaded.root));
}

/// `tynd build` preflight: print the open-scope warning. Best effort; a
/// config that fails to load is reported by the build itself.
pub(in crate::commands) fn warn_open_scopes(cwd: &Path) {
    let Ok(root) = crate::fsx::absolute(cwd) else {
        return;
    };
    let Ok(loaded) = config::load_typed(&root, None) else {
        return;
    };
    if let Some(message) = open_scope_warning(&loaded.config, &loaded.root) {
        crate::output::warn(message);
    }
}

fn open_scope_warning(config: &TyndConfig, root: &Path) -> Option<String> {
    let mut uses = declared_uses(config);
    let imports = scoped_import_pattern();
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
        if sets_security_policy(&source) {
            return None;
        }
        for capture in imports.captures_iter(&source) {
            let api = &capture[1];
            if let Some(api) = SCOPED_APIS.iter().find(|candidate| **candidate == api) {
                push_unique(&mut uses, (*api).to_owned());
            }
        }
    }
    if uses.is_empty() {
        return None;
    }
    Some(format!(
        "app uses {} but configures no security policy; resource scopes are open by default. \
         Set `security` in app.start({{ security: {{ fs, http, ... }} }}) to restrict access \
         (`defaultDeny: true` denies unlisted resources)",
        uses.join(", ")
    ))
}

/// Scope-gated APIs and unknown plugins the config itself declares.
fn declared_uses(config: &TyndConfig) -> Vec<String> {
    let mut uses = Vec::new();
    for api in &config.apis {
        if SCOPED_APIS.contains(&api.as_str()) {
            push_unique(&mut uses, api.clone());
        }
    }
    for plugin in &config.plugins {
        let package = plugin.package();
        match API_PLUGIN_PACKAGES.iter().find(|(_, pkg)| *pkg == package) {
            Some((api, _)) if SCOPED_APIS.contains(api) => {
                push_unique(&mut uses, (*api).to_owned());
            },
            Some(_) => {},
            // Not an official API plugin: it runs native code of its own.
            None => push_unique(&mut uses, format!("plugin {package}")),
        }
    }
    uses
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// Minimal JS token, enough to read an object literal's top-level keys.
#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    /// A string or template literal body.
    Text(String),
    Punct(char),
}

/// Tokenize `source`, dropping comments and whitespace and keeping string
/// literals opaque, so text inside them can never look like code.
fn tokenize(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '/' && regex_allowed(tokens.last()) {
            if let Some(end) = regex_literal_end(&chars, i) {
                i = end;
                tokens.push(Token::Text(String::new()));
            } else {
                tokens.push(Token::Punct(c));
                i += 1;
            }
        } else if matches!(c, '"' | '\'' | '`') {
            i += 1;
            let mut text = String::new();
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' {
                    i += 1;
                }
                if let Some(ch) = chars.get(i) {
                    text.push(*ch);
                }
                i += 1;
            }
            i += 1;
            tokens.push(Token::Text(text));
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            tokens.push(Token::Word(chars[start..i].iter().collect()));
        } else {
            if !c.is_whitespace() {
                tokens.push(Token::Punct(c));
            }
            i += 1;
        }
    }
    tokens
}

/// Whether a `/` after `previous` starts a regex literal rather than a
/// division: at the start, after an operator or opening/separating
/// punctuation, or after a keyword such as `return`.
fn regex_allowed(previous: Option<&Token>) -> bool {
    match previous {
        None => true,
        Some(Token::Text(_)) => false,
        Some(Token::Punct(p)) => !matches!(p, ')' | ']'),
        Some(Token::Word(word)) => matches!(
            word.as_str(),
            "return"
                | "typeof"
                | "case"
                | "in"
                | "of"
                | "delete"
                | "void"
                | "throw"
                | "new"
                | "else"
                | "do"
                | "yield"
                | "await"
                | "instanceof"
        ),
    }
}

/// Index just past the regex literal (flags included) opening at `chars[start]`,
/// or `None` when no closing `/` exists on the same line (then it was not a
/// regex after all).
fn regex_literal_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut in_class = false;
    while let Some(&c) = chars.get(i) {
        match c {
            '\n' | '\r' => return None,
            '\\' => i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                i += 1;
                while chars.get(i).is_some_and(|f| f.is_alphanumeric()) {
                    i += 1;
                }
                return Some(i);
            },
            _ => {},
        }
        i += 1;
    }
    None
}

/// A scoped API reached through its host-backed plugin, in any import form
/// (static, dynamic, `require`, re-export): `@tynd/plugin-<api>[/client]`.
/// `@tynd/core` exports none of them.
fn scoped_import_pattern() -> Regex {
    Regex::new(r#"["']@tynd/plugin-([a-z]+)(?:/client)?["']"#)
        .expect("static import pattern is valid")
}

/// True when `source` calls `app.start(...)` (the `app` export of
/// `@tynd/core`, or a local alias of it) with a config that sets a
/// `security` key: `{ security: ... }`, `{ security }`, `{ "security": ... }`,
/// or an identifier bound to such an object literal in the same file. Anything
/// that cannot be resolved statically (spread, imported or computed config)
/// counts as configured, since the absence of a policy cannot be shown.
fn sets_security_policy(source: &str) -> bool {
    let tokens = tokenize(source);
    let receivers = app_bindings(&tokens);
    (1..tokens.len().saturating_sub(3)).any(|i| {
        matches!(
            (&tokens[i], &tokens[i + 1], &tokens[i + 2]),
            (Token::Punct('.'), Token::Word(name), Token::Punct('('))
                if name == "start"
        ) && matches!(&tokens[i - 1], Token::Word(receiver) if receivers.contains(receiver))
            && start_argument_sets_security(&tokens, i + 3)
    })
}

/// Local names the `app` export is reachable under: `app` itself, plus
/// `import { app as x }` and `const { app: x } = ...` renames.
fn app_bindings(tokens: &[Token]) -> Vec<String> {
    let mut names = vec!["app".to_owned()];
    for window in tokens.windows(3) {
        if let [Token::Word(app), Token::Word(as_kw), Token::Word(alias)] = window {
            if app == "app" && as_kw == "as" {
                names.push(alias.clone());
            }
        }
        if let [Token::Word(app), Token::Punct(':'), Token::Word(alias)] = window {
            if app == "app" {
                names.push(alias.clone());
            }
        }
    }
    names
}

fn start_argument_sets_security(tokens: &[Token], arg: usize) -> bool {
    match (tokens.get(arg), tokens.get(arg + 1)) {
        (Some(Token::Punct('{')), _) => object_sets_security(tokens, arg),
        (Some(Token::Punct(')') | Token::Text(_)) | None, _) => false,
        (Some(Token::Word(literal)), _) if literal.starts_with(|c: char| c.is_ascii_digit()) => {
            false
        },
        (Some(Token::Word(name)), Some(Token::Punct(')' | ','))) => {
            bound_object_sets_security(tokens, name)
        },
        _ => true,
    }
}

/// `const|let|var <name> [: Type] = { ... }` anywhere in the file.
fn bound_object_sets_security(tokens: &[Token], name: &str) -> bool {
    for (j, token) in tokens.iter().enumerate() {
        let is_decl =
            matches!(token, Token::Word(kw) if matches!(kw.as_str(), "const" | "let" | "var"));
        if !is_decl || !matches!(tokens.get(j + 1), Some(Token::Word(n)) if n == name) {
            continue;
        }
        let eq =
            (j + 2..tokens.len().min(j + 12)).find(|k| matches!(tokens[*k], Token::Punct('=')));
        return match eq {
            Some(k) if matches!(tokens.get(k + 1), Some(Token::Punct('{'))) => {
                object_sets_security(tokens, k + 1)
            },
            _ => true,
        };
    }
    true
}

/// Whether the object literal opening at `tokens[open]` has a top-level
/// `security` key (or a spread, which may bring one).
fn object_sets_security(tokens: &[Token], open: usize) -> bool {
    let mut depth = 1usize;
    let mut entry_start = true;
    for i in open + 1..tokens.len() {
        if depth == 1 && entry_start {
            entry_start = false;
            match &tokens[i] {
                Token::Word(key) | Token::Text(key)
                    if key == "security"
                        && matches!(tokens.get(i + 1), Some(Token::Punct(':' | ',' | '}'))) =>
                {
                    return true;
                },
                Token::Punct('.') => return true,
                _ => {},
            }
        }
        match &tokens[i] {
            Token::Punct('{' | '(' | '[') => depth += 1,
            Token::Punct('}' | ')' | ']') => {
                depth -= 1;
                if depth == 0 {
                    return false;
                }
            },
            Token::Punct(',') if depth == 1 => entry_start = true,
            _ => {},
        }
    }
    false
}

#[cfg(test)]
mod tests;
