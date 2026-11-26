//! `cli` OS API — surface the host's argv to the frontend.
//!
//! Manual parser (no `clap` dep) supporting the four shapes apps use:
//! `--key=value`, `--key value`, `--flag`, positional.
//!
//! `argv` is snapshotted on first call and cached.

use std::{collections::BTreeMap, sync::OnceLock};

use serde::{Deserialize, Serialize};

use crate::api::HostApi;
use crate::tynd;

/// One flag value. Untagged enum so JSON stays scalar (`string | boolean`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum FlagValue {
    /// `--flag` (no value) → JSON `true`.
    Bool(bool),
    /// `--key=value` or `--key value` → JSON string.
    Str(String),
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CliMatches {
    /// Positional args (no flag, no binary path, no subcommand).
    pub(crate) args: Vec<String>,
    /// Repeated keys overwrite. `BTreeMap` for deterministic test serialization.
    pub(crate) flags: BTreeMap<String, FlagValue>,
    /// First non-flag arg after the binary path.
    pub(crate) subcommand: Option<String>,
    /// Raw argv snapshot (includes `args[0]`).
    pub(crate) raw: Vec<String>,
}

static CACHED: OnceLock<CliMatches> = OnceLock::new();

fn matches_cached() -> &'static CliMatches {
    CACHED.get_or_init(|| {
        let argv: Vec<String> = std::env::args().collect();
        parse(&argv)
    })
}

/// Dispatch marker — argv state lives in `CACHED`.
#[derive(Debug, Default)]
pub(crate) struct CliApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("cli", |method, args| dispatch(method, &args));

#[tynd::handler(api = "cli")]
impl CliApi {
    /// Return the parsed `{ args, flags, subcommand, raw }` snapshot
    /// of the host process's argv.
    pub fn matches() -> CliMatches {
        matches_cached().clone()
    }

    /// Return the raw `std::env::args` snapshot (including `args[0]`,
    /// the binary path).
    pub fn raw() -> Vec<String> {
        matches_cached().raw.clone()
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
            // Short flag — same shapes as long. POSIX bundling is not
            // unpacked: `-rf` stays a single `rf` key.
            if rest.is_empty() {
                // Bare `-` is conventionally stdin / stdout — keep as
                // a positional so apps that care can route on it.
                args.push(token.clone());
                continue;
            }
            insert_flag(&mut flags, &mut iter, rest);
        } else if subcommand.is_none() {
            // The very first positional becomes the subcommand.
            subcommand = Some(token.clone());
        } else {
            args.push(token.clone());
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
/// token only when the next token does not start with `-` (a new flag,
/// `-` or `--`).
fn insert_flag<'a, I: Iterator<Item = &'a String>>(
    flags: &mut BTreeMap<String, FlagValue>,
    iter: &mut std::iter::Peekable<I>,
    rest: &str,
) {
    if let Some((key, value)) = rest.split_once('=') {
        flags.insert(key.to_string(), FlagValue::Str(value.to_string()));
    } else if iter.peek().is_some_and(|next| !next.starts_with('-')) {
        let value = iter.next().expect("peek confirmed presence").clone();
        flags.insert(rest.to_string(), FlagValue::Str(value));
    } else {
        flags.insert(rest.to_string(), FlagValue::Bool(true));
    }
}
