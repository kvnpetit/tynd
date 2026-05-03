//! `cli` OS API — surface the host process's command-line arguments to
//! the frontend.
//!
//! Tynd apps often want to react to the way they were launched: a
//! `--port=3000` switch routing the app to a custom dev server, a path
//! argument opening a specific file, a `--headless` flag toggling the
//! window. Without this API the frontend has no view onto `argv`
//! (the OS hands it to the host, not to the webview).
//!
//! Parsing is intentionally **manual** rather than `clap`-based: clap
//! pulls a few hundred KiB into every binary and forces a schema
//! definition the user has to maintain in two places. The parser below
//! is ~70 lines and supports the four shapes apps actually use
//! (`--key=value`, `--key value`, `--flag`, positional).
//!
//! ## Caching
//!
//! `argv` is captured once on the first call (snapshot of
//! `std::env::args`) and the parsed shape is computed lazily, then
//! reused for every subsequent dispatch. A subsequent process restart
//! picks up the new args naturally; nothing inside a single process
//! changes `argv` at runtime worth re-snapshotting.

use std::{collections::BTreeMap, sync::OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ipc_error::IpcError;

/// Wire-shape for a single flag value: either a string (`--name=value`,
/// `--name value`) or a boolean (`--flag` with no companion value).
/// Modeled as an untagged enum so the JSON form stays scalar — matches
/// what TypeScript callers expect (`string | boolean`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FlagValue {
    /// `--flag` (no companion value) — emitted as JSON `true`.
    Bool(bool),
    /// `--key=value` or `--key value` — emitted as a JSON string.
    Str(String),
}

/// Parsed shape returned by `matches`.
#[derive(Debug, Clone, Serialize)]
pub struct CliMatches {
    /// Positional args (everything that isn't a flag, the binary path,
    /// or the subcommand). Order preserved.
    pub args: Vec<String>,
    /// `--name=value` → `{ name: "value" }`, `--flag` → `{ flag: true }`,
    /// `--name value` → `{ name: "value" }`. Repeated keys overwrite.
    /// `BTreeMap` for deterministic ordering when serialised — useful
    /// for tests and snapshots.
    pub flags: BTreeMap<String, FlagValue>,
    /// First non-flag arg after the binary path, when present.
    pub subcommand: Option<String>,
    /// Untouched argv (`std::env::args` snapshot, including `args[0]`).
    pub raw: Vec<String>,
}

static CACHED: OnceLock<CliMatches> = OnceLock::new();

/// Lazily parse the process argv once and return a borrowed snapshot.
/// Parse cost is paid by the first caller; everyone else hits the
/// `OnceLock` fast path.
fn matches() -> &'static CliMatches {
    CACHED.get_or_init(|| {
        let argv: Vec<String> = std::env::args().collect();
        parse(&argv)
    })
}

pub fn dispatch(method: &str, _args: &Value) -> Result<Value, String> {
    match method {
        "matches" => serde_json::to_value(matches())
            .map_err(|e| IpcError::new("cli", "matches", e).into_string()),
        "raw" => serde_json::to_value(&matches().raw)
            .map_err(|e| IpcError::new("cli", "raw", e).into_string()),
        _ => Err(IpcError::unknown_method("cli", method).into_string()),
    }
}

/// Parse argv into the four-bucket shape. Pure function so unit tests
/// don't need to mutate the process-wide `OnceLock`.
fn parse(argv: &[String]) -> CliMatches {
    let raw = argv.to_vec();
    let mut args = Vec::new();
    let mut flags: BTreeMap<String, FlagValue> = BTreeMap::new();
    let mut subcommand: Option<String> = None;

    // Skip args[0] (binary path / argv0) — apps never see it as a
    // user-facing argument.
    let mut iter = argv.iter().skip(1).peekable();
    let mut first_positional_seen = false;

    while let Some(token) = iter.next() {
        if let Some(rest) = token.strip_prefix("--") {
            // Long flag: `--key=value` or `--key [value]` or `--flag`
            if rest.is_empty() {
                // `--` is the conventional "end of options" marker.
                // Everything after lands in positional args verbatim.
                for tail in iter.by_ref() {
                    args.push(tail.clone());
                }
                break;
            }
            insert_flag(&mut flags, &mut iter, rest);
        } else if let Some(rest) = token.strip_prefix('-') {
            // Short flag — same shape as long, no `=` splitting (POSIX
            // bundling like `-rf` left intact as a single key, value
            // bundles can be unpacked by the user if needed).
            if rest.is_empty() {
                // Bare `-` is conventionally stdin / stdout — keep as
                // a positional so apps that care can route on it.
                args.push(token.clone());
                continue;
            }
            insert_flag(&mut flags, &mut iter, rest);
        } else {
            // Positional. The very first non-flag becomes the
            // subcommand; everything else lands in `args`.
            if first_positional_seen {
                args.push(token.clone());
            } else {
                first_positional_seen = true;
                subcommand = Some(token.clone());
            }
        }
    }

    CliMatches {
        args,
        flags,
        subcommand,
        raw,
    }
}

/// Shared logic for both `--long` and `-short` flags: `key=value`,
/// `key value`, or bare `key` (boolean true). Walks `iter` ahead by one
/// token only when the next token does not start a new flag.
fn insert_flag<'a, I: Iterator<Item = &'a String>>(
    flags: &mut BTreeMap<String, FlagValue>,
    iter: &mut std::iter::Peekable<I>,
    rest: &str,
) {
    if let Some((key, value)) = rest.split_once('=') {
        flags.insert(key.to_string(), FlagValue::Str(value.to_string()));
    } else if iter
        .peek()
        .is_some_and(|next| !next.starts_with('-') && next.as_str() != "--")
    {
        let value = iter.next().expect("peek confirmed presence").clone();
        flags.insert(rest.to_string(), FlagValue::Str(value));
    } else {
        flags.insert(rest.to_string(), FlagValue::Bool(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    fn flag_str<'a>(m: &'a CliMatches, key: &str) -> Option<&'a str> {
        match m.flags.get(key) {
            Some(FlagValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    fn flag_bool(m: &CliMatches, key: &str) -> Option<bool> {
        match m.flags.get(key) {
            Some(FlagValue::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    #[test]
    fn parses_subcommand_flag_and_positional() {
        let m = parse(&argv(&["app", "build", "--target=release", "src/main.ts"]));
        assert_eq!(m.subcommand.as_deref(), Some("build"));
        assert_eq!(flag_str(&m, "target"), Some("release"));
        assert_eq!(m.args, vec!["src/main.ts".to_string()]);
        assert_eq!(m.raw.len(), 4);
    }

    #[test]
    fn empty_argv_yields_empty_matches() {
        let m = parse(&argv(&["app"]));
        assert!(m.subcommand.is_none());
        assert!(m.flags.is_empty());
        assert!(m.args.is_empty());
    }

    #[test]
    fn boolean_flag_no_value() {
        let m = parse(&argv(&["app", "--verbose"]));
        assert_eq!(flag_bool(&m, "verbose"), Some(true));
    }

    #[test]
    fn space_separated_value() {
        let m = parse(&argv(&["app", "--name", "alice"]));
        assert_eq!(flag_str(&m, "name"), Some("alice"));
    }

    #[test]
    fn double_dash_terminates_flag_parsing() {
        let m = parse(&argv(&["app", "build", "--", "--not-a-flag", "x"]));
        assert_eq!(m.subcommand.as_deref(), Some("build"));
        assert_eq!(m.args, vec!["--not-a-flag".to_string(), "x".to_string()]);
        assert!(m.flags.is_empty());
    }

    #[test]
    fn short_flag_with_value() {
        let m = parse(&argv(&["app", "-p", "8080"]));
        assert_eq!(flag_str(&m, "p"), Some("8080"));
    }

    #[test]
    fn bare_dash_treated_as_positional() {
        let m = parse(&argv(&["app", "cat", "-"]));
        assert_eq!(m.subcommand.as_deref(), Some("cat"));
        assert_eq!(m.args, vec!["-".to_string()]);
    }

    #[test]
    fn dispatch_raw_returns_full_argv() {
        // Can't override the cache from tests (OnceLock); just exercise
        // the dispatch code path and confirm shape.
        let v = dispatch("raw", &json!(null)).unwrap();
        assert!(v.is_array());
    }

    #[test]
    fn dispatch_matches_returns_object_shape() {
        let v = dispatch("matches", &json!(null)).unwrap();
        assert!(v.is_object());
        assert!(v.get("args").is_some());
        assert!(v.get("flags").is_some());
        assert!(v.get("raw").is_some());
    }

    #[test]
    fn unknown_method_errors() {
        let err = dispatch("wat", &json!(null)).unwrap_err();
        assert!(err.contains("unknown method"));
    }
}
