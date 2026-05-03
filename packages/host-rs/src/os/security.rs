//! Capability-based allow/deny lists for FS + HTTP. Opt-in — until the app
//! calls `security.configure`, every API behaves as before.
//!
//! Patterns are simple globs: `*` matches any run of characters within a
//! segment, `**` crosses separators. Matching is case-insensitive on
//! Windows for FS, case-sensitive everywhere else.
//!
//! Precedence: deny overrides allow. An empty `allow` list means "allow
//! everything" unless `defaultDeny` is set, in which case it means "deny
//! everything".
//!
//! # Persisted grants (folded from `@tynd/plugin-persisted-scope`)
//!
//! In addition to the live policy, we persist a JSON file under
//! `<data_dir>/<exe_stem>/security-persisted-grants.json` so that paths
//! the user grants at runtime (typically through a folder picker) survive
//! across launches without re-prompting. The on-disk shape:
//!
//! ```json
//! { "fs": ["~/Documents", "/Users/jane/Project"], "http": ["https://api.example.com"] }
//! ```
//!
//! Tilde (`~/foo`) entries are kept verbatim on disk so the file stays
//! portable across machines / users; expansion happens at load time
//! (because the policy globs match against absolute paths).
//!
//! Auto-load fires lazily on the first `configure(...)` /
//! `add_*_allow(...)` call (or first `persist_grant(...)`) — there's no
//! explicit boot hook, callers just see the persisted entries already in
//! the live allow-list.

use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
};

use arc_swap::ArcSwap;
use globset::{GlobSet, GlobSetBuilder};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::ipc_error::IpcError;

#[derive(Debug, Default, Deserialize, Type)]
pub struct PolicyInput {
    #[serde(default)]
    pub fs: ScopeInput,
    #[serde(default)]
    pub http: ScopeInput,
    /// When true, an empty `allow` list denies every call of that scope.
    /// Default is false (open-by-default).
    #[serde(default, rename = "defaultDeny")]
    pub default_deny: bool,
    /// Commands the app is willing to execute via `process.execShell`.
    /// Until this list is non-empty, every `execShell` call is rejected
    /// — `process.exec` (with separate `args`) is always available and
    /// is the safe path. Each entry is matched against the first
    /// whitespace-delimited token of the shell command (before any
    /// metacharacter); the rest of the line is then re-checked for
    /// shell metacharacters and refused if any are present.
    #[serde(default, rename = "shellExecAllowedCommands")]
    pub shell_exec_allowed_commands: Vec<String>,
}

#[derive(Debug, Default, Deserialize, Type)]
pub struct ScopeInput {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Aggregated allow/deny scope. The raw `Vec<String>` mirrors the
/// deployer-provided patterns so we can serialize / mutate / introspect
/// them; the `GlobSet`s are derived caches that the hot `check_*` path
/// queries instead of re-compiling individual `Glob`s on every call.
///
/// Performance: a `GlobSet` lookup is O(1) on the strategy hash + a
/// regex DFA scan vs. the previous O(N) `for pat in allow {
/// Glob::new(pat).compile_matcher().is_match(...) }` which spent the
/// bulk of its time inside `regex_syntax::Parser::parse`. For a typical
/// 32-glob policy this drops a per-call cost from ~80 µs to <500 ns —
/// 50-200× depending on glob complexity.
///
/// Cache invariants:
/// - `allow_set` matches the current `allow` vector (same length, same
///   order, same patterns). `deny_set` mirrors `deny`.
/// - Every mutation site (`configure`, `add_*_allow`, `remove_*_allow`,
///   `auto_restore_persisted`) MUST call `rebuild_*` after touching the
///   raw vector. Tests that hand-build a `Policy` use the
///   `Scope::new(...)` constructor which compiles in one shot.
/// - A pattern that fails to compile is silently dropped from the cache
///   (matches `globset::Glob::new` returning `Err`). The previous
///   implementation also silently dropped these via `is_ok_and`, so the
///   contract stays the same.
#[derive(Debug, Default, Clone)]
struct Scope {
    allow: Vec<String>,
    deny: Vec<String>,
    allow_set: GlobSet,
    deny_set: GlobSet,
}

impl Scope {
    /// Build a scope from raw allow/deny vectors. The `GlobSet`s are
    /// compiled once here so callers don't have to remember to invoke
    /// `rebuild_*` separately. Used by `configure(...)` (one shot) and
    /// the `#[cfg(test)]` policy installer in this module.
    fn new(allow: Vec<String>, deny: Vec<String>) -> Self {
        let allow_set = compile_glob_set(&allow);
        let deny_set = compile_glob_set(&deny);
        Self {
            allow,
            deny,
            allow_set,
            deny_set,
        }
    }

    /// Recompile the cached `allow_set` after mutating `allow`. Cheap
    /// at small sizes (~µs per pattern) — called from
    /// `add_*_allow` / `remove_*_allow`.
    fn rebuild_allow(&mut self) {
        self.allow_set = compile_glob_set(&self.allow);
    }

    /// Recompile the cached `deny_set`. No mutation API exposes deny
    /// today, but `configure` rebuilds via `Scope::new`; the helper is
    /// here for symmetry and future-proofing.
    #[allow(dead_code)]
    fn rebuild_deny(&mut self) {
        self.deny_set = compile_glob_set(&self.deny);
    }
}

/// Compile a list of pattern strings into a single `GlobSet`. Patterns
/// that fail to parse are dropped — see [`Scope`] doc for the rationale.
/// Patterns are normalized the same way the input is in [`canonicalize_fs`]
/// (Windows `\\` → `/` + lowercase) so a rule written
/// `C:\Users\*\Desktop` still matches `c:/users/jane/desktop` on the
/// hot path.
fn compile_glob_set(patterns: &[String]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let normalized = if cfg!(target_os = "windows") {
            pat.replace('\\', "/").to_lowercase()
        } else {
            pat.clone()
        };
        // `literal_separator(true)` keeps `*` confined to a single path
        // segment — only `**` crosses `/`. Same contract as the old
        // hand-rolled matcher and the `matches(...)` fallback below.
        if let Ok(glob) = globset::GlobBuilder::new(&normalized)
            .literal_separator(true)
            .build()
        {
            builder.add(glob);
        }
    }
    builder.build().unwrap_or_else(|_| GlobSet::empty())
}


#[derive(Debug, Default, Clone)]
struct Policy {
    fs: Scope,
    http: Scope,
    default_deny: bool,
    configured: bool,
    /// Whitelist for `process.execShell` (see `PolicyInput::shell_exec_allowed_commands`).
    shell_exec_allowed: Vec<String>,
}

/// Lock-free read of the current policy. Writes go through
/// `with_policy_write()` which serializes RMW updates under a small
/// `Mutex` and publishes the new state via `ArcSwap::store(...)`.
/// Readers compile down to a single atomic load + Arc bump — no
/// `RwLock::read` contention even when 20+ threads call `check_fs`
/// concurrently. `LazyLock` (Rust 1.80+) builds the inner `ArcSwap`
/// on first deref so callers reach it via direct `POLICY.load()`
/// instead of a `policy()` accessor wrapping `OnceLock::get_or_init`.
static POLICY: LazyLock<ArcSwap<Policy>> =
    LazyLock::new(|| ArcSwap::from_pointee(Policy::default()));
/// Writer-only mutex. Keeps RMW (`add_fs_allow` / `remove_fs_allow` /
/// `auto_restore_persisted`) atomic without forcing readers through a
/// lock. Held only on the slow path.
static POLICY_WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Lock the writer mutex and clone the current policy into an owned
/// `Policy` so the caller can mutate + publish via `POLICY.store(...)`.
/// Returns the guard so the caller's drop releases the writer lock at
/// the right point in the call stack.
fn with_policy_write<R>(f: impl FnOnce(&mut Policy) -> R) -> R {
    let _guard = POLICY_WRITE_LOCK.lock();
    let current = POLICY.load_full();
    // `Arc::try_unwrap` would let us avoid the clone when no reader
    // holds a `load_full()` snapshot, but the contention pattern here
    // (one writer, many readers) means it's almost always cheaper to
    // pay the clone unconditionally than to retry on contention.
    let mut next = (*current).clone();
    let r = f(&mut next);
    POLICY.store(Arc::new(next));
    r
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "configure" => configure(args),
        "isFsAllowed" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "isFsAllowed", "path")))?;
            Ok(Value::Bool(check_fs(path).is_ok()))
        },
        "isHttpAllowed" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "isHttpAllowed", "url")))?;
            Ok(Value::Bool(check_http(url).is_ok()))
        },
        // ----- W2.2 — live policy mutation -------------------------------
        // The legacy `configure` REPLACES the entire policy; plugins like
        // `persisted-scope` need to extend the live allow-list with paths
        // the user just granted via a folder picker, without forcing the
        // app to re-call `configure(...)` from scratch (which would race
        // against in-flight FS calls on a different thread). Each helper
        // returns `true` when the entry was newly added / removed so the
        // caller can short-circuit on idempotent calls.
        "addFsAllow" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "addFsAllow", "path")))?;
            Ok(Value::Bool(add_fs_allow(path)))
        },
        "removeFsAllow" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "removeFsAllow", "path")))?;
            Ok(Value::Bool(remove_fs_allow(path)))
        },
        "addHttpAllow" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "addHttpAllow", "url")))?;
            Ok(Value::Bool(add_http_allow(url)))
        },
        "removeHttpAllow" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "removeHttpAllow", "url")))?;
            Ok(Value::Bool(remove_http_allow(url)))
        },
        // ----- Persisted grants (folded from `@tynd/plugin-persisted-scope`) ----
        // These extend the live policy AND remember the grant on disk so
        // a user-picked folder survives the next launch without re-
        // prompting. `kind` defaults to `"fs"` to match the JS surface.
        "persistGrant" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "persistGrant", "path")))?;
            let kind = parse_grant_kind(args)?;
            persist_grant(path, kind)
                .map(Value::Bool)
                .map_err(|e| String::from(IpcError::new("security", "persistGrant", e)))
        },
        "revokePersisted" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "revokePersisted", "path")))?;
            let kind = parse_grant_kind(args)?;
            revoke_persisted(path, kind)
                .map(Value::Bool)
                .map_err(|e| String::from(IpcError::new("security", "revokePersisted", e)))
        },
        "listPersisted" => {
            let kind = parse_grant_kind(args)?;
            Ok(Value::Array(
                list_persisted(kind)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ))
        },
        "hasPersisted" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from(IpcError::missing_arg("security", "hasPersisted", "path")))?;
            let kind = parse_grant_kind(args)?;
            Ok(Value::Bool(has_persisted(path, kind)))
        },
        "clearPersisted" => {
            let kind = parse_grant_kind(args)?;
            clear_persisted(kind)
                .map(|()| Value::Null)
                .map_err(|e| String::from(IpcError::new("security", "clearPersisted", e)))
        },
        _ => Err(String::from(IpcError::unknown_method("security", method))),
    }
}

/// Parse the `kind` argument shared by every `*Persisted*` dispatch arm.
/// Defaults to `Fs` when absent or not a string. Returns an error only on
/// an explicitly invalid value (e.g. `"filesystem"`).
fn parse_grant_kind(args: &Value) -> Result<GrantKind, String> {
    match args.get("kind").and_then(Value::as_str) {
        None | Some("fs") => Ok(GrantKind::Fs),
        Some("http") => Ok(GrantKind::Http),
        Some(other) => Err(format!(
            "security: invalid 'kind' — expected 'fs' or 'http', got '{other}'"
        )),
    }
}

/// Add a path glob to the live `fs.allow` list. Returns `true` when the
/// entry was newly added, `false` when it was already present (idempotent).
///
/// SECURITY: this is the only sanctioned way to extend the policy at
/// runtime — it preserves every other field (`deny`, `default_deny`,
/// `shell_exec_allowed`, `configured`) verbatim, swap is atomic under
/// the same `RwLock` `check_fs` reads. Used by the persisted-grants
/// auto-restore so user-granted folders survive across launches without
/// forcing the app to rebuild + re-call `security.configure({ ... })`.
///
/// Once you've called this, the policy becomes "configured" — every
/// subsequent `check_fs` call enforces the allow-list (whereas an
/// unconfigured policy fails open). Adding a path to a previously-
/// unconfigured policy therefore flips the gate ON.
pub fn add_fs_allow(path: &str) -> bool {
    // Lazy auto-restore on the first allow-mutation call. Idempotent:
    // subsequent calls see the latch flipped and short-circuit before
    // touching disk.
    auto_restore_persisted();
    with_policy_write(|p| {
        if p.fs.allow.iter().any(|existing| existing == path) {
            // Still mark as configured so a no-op `add` after `configure`
            // doesn't leave a stale "open by default" state if the caller
            // intended to switch into ACL mode.
            p.configured = true;
            return false;
        }
        p.fs.allow.push(path.to_string());
        p.fs.rebuild_allow();
        p.configured = true;
        true
    })
}

/// Remove a path glob from the live `fs.allow` list. Returns `true` when
/// an entry was actually removed, `false` when nothing matched.
pub fn remove_fs_allow(path: &str) -> bool {
    with_policy_write(|p| {
        let before = p.fs.allow.len();
        p.fs.allow.retain(|existing| existing != path);
        let changed = before != p.fs.allow.len();
        if changed {
            p.fs.rebuild_allow();
        }
        changed
    })
}

/// HTTP twin of [`add_fs_allow`]. Same semantics — newly-added vs. already-
/// present, atomic swap under the same lock the `check_http` hot path
/// reads.
pub fn add_http_allow(url: &str) -> bool {
    auto_restore_persisted();
    with_policy_write(|p| {
        if p.http.allow.iter().any(|existing| existing == url) {
            p.configured = true;
            return false;
        }
        p.http.allow.push(url.to_string());
        p.http.rebuild_allow();
        p.configured = true;
        true
    })
}

/// HTTP twin of [`remove_fs_allow`].
pub fn remove_http_allow(url: &str) -> bool {
    with_policy_write(|p| {
        let before = p.http.allow.len();
        p.http.allow.retain(|existing| existing != url);
        let changed = before != p.http.allow.len();
        if changed {
            p.http.rebuild_allow();
        }
        changed
    })
}

fn configure(args: &Value) -> Result<Value, String> {
    let input: PolicyInput = serde_json::from_value(args.clone())
        .map_err(|e| format!("security.configure: invalid shape — {e}"))?;
    // Merge persisted grants into the deployer-provided allow lists so
    // user-granted folders survive a `configure(...)` reset (which is
    // otherwise REPLACE semantics). Tilde entries are expanded + the FS
    // entries grow a `/**` suffix to cover the user-picked subtree.
    let persisted = load_persisted_unlocked();
    let mut fs_allow = input.fs.allow;
    for entry in &persisted.fs {
        let absolute = expand_tilde(entry);
        let pattern = format!("{}/**", absolute.trim_end_matches('/'));
        if !fs_allow.iter().any(|p| p == &pattern) {
            fs_allow.push(pattern);
        }
    }
    let mut http_allow = input.http.allow;
    for entry in &persisted.http {
        if !http_allow.iter().any(|p| p == entry) {
            http_allow.push(entry.clone());
        }
    }
    let next = Policy {
        fs: Scope::new(fs_allow, input.fs.deny),
        http: Scope::new(http_allow, input.http.deny),
        default_deny: input.default_deny,
        configured: true,
        shell_exec_allowed: input.shell_exec_allowed_commands,
    };
    // `configure` REPLACES — no RMW needed, just publish the new Arc.
    // Acquire the writer lock so an in-flight `add_*_allow(...)` doesn't
    // race with the replacement (it would otherwise see its RMW snapshot
    // get clobbered without errors).
    {
        let _guard = POLICY_WRITE_LOCK.lock();
        POLICY.store(Arc::new(next));
    }
    // Latch so subsequent `add_*_allow(...)` calls don't try to re-merge.
    PERSISTED.auto_restored.store(true, Ordering::Release);
    Ok(Value::Null)
}

pub fn check_fs(path: &str) -> Result<(), String> {
    // `load()` returns an inexpensive `Guard` — single atomic load, no
    // lock contention. Holds the snapshot for the duration of `check`.
    let p = POLICY.load();
    if !p.configured {
        return Ok(());
    }
    let canonical = canonicalize_fs(path);
    check(&canonical, &p.fs, p.default_deny, "fs")
}

pub fn check_http(url: &str) -> Result<(), String> {
    let p = POLICY.load();
    if !p.configured {
        return Ok(());
    }
    check(url, &p.http, p.default_deny, "http")
}

/// Shell metacharacters that let a command line spawn extra processes,
/// redirect IO, or expand into something the caller didn't intend. The
/// `process.shellExec` capability is double-gated: the first token must
/// appear in `shellExecAllowedCommands`, AND the rest of the line must
/// be free of these characters.
const SHELL_METACHARS: &[char] = &[
    '&', '|', ';', '<', '>', '(', ')', '{', '}', '\'', '"', '`', '\\', '$', '\n', '\r',
];

/// Validate `cmd_line` against the `shellExecAllowedCommands` whitelist.
///
/// SECURITY: shell execution requires explicit
/// `security.configure({ shellExecAllowedCommands: [...] })`. Without
/// it, all calls reject with `process.shellExec: not permitted`. Even
/// when a command is whitelisted, any shell metacharacter
/// (`&| ;<>(){}'"\\\`$\n\r`) in the command line is grounds for refusal
/// — the caller must use `process.exec` with separate `args` to compose
/// arguments safely.
pub fn check_shell_exec(cmd_line: &str) -> Result<(), String> {
    let p = POLICY.load();
    if p.shell_exec_allowed.is_empty() {
        return Err(
            "process.shellExec: not permitted — call \
             `security.configure({ shellExecAllowedCommands: [...] })` to opt in"
                .into(),
        );
    }
    let trimmed = cmd_line.trim_start();
    // First token = up to whitespace OR a shell metacharacter. Splitting
    // on both lets us surface the metachar error (more actionable) when
    // the caller wrote `ls; rm` rather than the misleading
    // "'ls;' is not whitelisted".
    let first_token = trimmed
        .split(|c: char| c.is_whitespace() || SHELL_METACHARS.contains(&c))
        .next()
        .unwrap_or("");
    if first_token.is_empty() {
        return Err("process.shellExec: empty command".into());
    }
    // Check metacharacters BEFORE the whitelist lookup so a whitelisted
    // program embedded in a hostile line (`ls; rm -rf /`) hits the
    // clearer "metacharacters are not allowed" error.
    if cmd_line.chars().any(|c| SHELL_METACHARS.contains(&c)) {
        return Err(
            "process.shellExec: shell metacharacters are not allowed — use `process.exec` with \
             separate `args` to compose arguments"
                .into(),
        );
    }
    if !p
        .shell_exec_allowed
        .iter()
        .any(|allowed| allowed == first_token)
    {
        return Err(format!(
            "process.shellExec: '{first_token}' is not in `shellExecAllowedCommands`"
        ));
    }
    Ok(())
}

fn check(candidate: &str, scope: &Scope, default_deny: bool, kind: &str) -> Result<(), String> {
    // Cached `GlobSet` lookups — single pass through the regex DFA per
    // call instead of one `Glob::new` compile per pattern. See `Scope`
    // doc for the perf rationale and cache invariants.
    if scope.deny_set.is_match(candidate) {
        return Err(format!("{kind}: denied by security policy — '{candidate}'"));
    }
    if scope.allow.is_empty() {
        if default_deny {
            return Err(format!(
                "{kind}: no allow rule matches '{candidate}' and defaultDeny is on"
            ));
        }
        return Ok(());
    }
    if scope.allow_set.is_match(candidate) {
        Ok(())
    } else {
        Err(format!("{kind}: no allow rule matches '{candidate}'"))
    }
}

/// Normalize path separators and case (on Windows) so `C:\foo\bar` matches
/// `C:/foo/*`. We don't canonicalize via the filesystem — that would reject
/// writes to not-yet-existing files.
fn canonicalize_fs(path: &str) -> String {
    let normalized = Path::new(path).to_string_lossy().replace('\\', "/");
    if cfg!(target_os = "windows") {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

/// Test-only single-pattern matcher. Production code goes through the
/// cached `Scope::allow_set` / `deny_set` `GlobSet`s (see [`Scope`] doc);
/// these inline helpers only stay around because the existing glob
/// regression tests were written before the cache landed and we don't
/// want to touch their assertion shape.
#[cfg(test)]
fn matches(pattern: &str, input: &str) -> bool {
    let pat = if cfg!(target_os = "windows") {
        pattern.replace('\\', "/").to_lowercase()
    } else {
        pattern.to_string()
    };
    // `literal_separator(true)` keeps `*` confined to a single path
    // segment — only `**` crosses `/`. Mirrors the previous matcher's
    // behaviour and is what the security policy contract documents.
    globset::GlobBuilder::new(&pat)
        .literal_separator(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(input))
}

// =====================================================================
// Persisted grants
// =====================================================================
//
// Folded from the former `@tynd/plugin-persisted-scope` crate. Stores
// user-granted FS / HTTP entries in a JSON file under `<data_dir>/<exe>`
// so a folder picked from a dialog isn't re-prompted on the next launch.
// Tilde-prefixed entries (`~/Documents`) are kept verbatim on disk for
// portability across machines / users; expansion happens at load time
// because the live policy globs are matched against absolute paths.

/// Discriminator on every persisted-grant API. Mirrors the JS
/// `"fs" | "http"` literal union the dispatch arms accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantKind {
    Fs,
    Http,
}

/// On-disk shape. `BTreeSet` keeps the JSON entries in stable order
/// (diff-friendly when the file lands in a synced folder) and dedupes
/// for free.
#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedGrants {
    #[serde(default)]
    fs: BTreeSet<String>,
    #[serde(default)]
    http: BTreeSet<String>,
}

/// Process-global state: the in-memory mirror of the JSON file plus a
/// "have we already auto-restored?" latch so the first
/// `add_*_allow(...)` / `configure(...)` call hydrates the live policy
/// and subsequent calls short-circuit. Wrapped behind an accessor so the
/// `OnceLock` initializer reads disk exactly once per process.
struct PersistedState {
    grants: RwLock<PersistedGrants>,
    storage_path: RwLock<Option<PathBuf>>,
    auto_restored: AtomicBool,
}

/// Process-wide persisted-grants state. `LazyLock` (Rust 1.80+) reads
/// the JSON file on first deref so callers reach it via direct
/// `PERSISTED.grants.read()` — replaces the previous `PERSISTED`
/// accessor wrapping `OnceLock::get_or_init`.
static PERSISTED: LazyLock<PersistedState> = LazyLock::new(|| {
    let path = default_persisted_path();
    let grants = path
        .as_ref()
        .and_then(|p| match fs::read_to_string(p) {
            Ok(raw) => Some(serde_json::from_str::<PersistedGrants>(&raw).unwrap_or_default()),
            // Missing or corrupted file = empty state. We never
            // panic here — startup must be resilient against a
            // freshly-installed app, a mid-write crash, or a hostile
            // edit of the JSON.
            Err(e) if e.kind() == ErrorKind::NotFound => None,
            Err(_) => None,
        })
        .unwrap_or_default();
    PersistedState {
        grants: RwLock::new(grants),
        storage_path: RwLock::new(path),
        auto_restored: AtomicBool::new(false),
    }
});

/// `<data_dir>/<exe_stem>/security-persisted-grants.json`. Returns
/// `None` only on the rare host where `dirs::data_dir()` returns nothing
/// — in which case the grants module still works in memory but doesn't
/// persist across launches.
fn default_persisted_path() -> Option<PathBuf> {
    let base = dirs::data_dir()?;
    let app_id = persisted_exe_stem();
    Some(base.join(app_id).join("security-persisted-grants.json"))
}

/// Same shape `os::notification::exe_stem` uses to derive an app id.
/// Falls back to a stable string so two co-installed apps without a
/// proper executable name still get distinct (but predictable) folders.
fn persisted_exe_stem() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd-app".to_string())
}

/// Expand a leading `~` to the user's home directory. Returned for
/// callers that want the canonical absolute path before feeding the
/// entry into the live security policy. Other paths and entries without
/// a tilde are returned unchanged.
#[must_use]
pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

/// Read a snapshot of the persisted grants without touching the live
/// policy. Used by `configure(...)` to fold persisted entries into the
/// fresh policy under a single write lock.
fn load_persisted_unlocked() -> PersistedGrants {
    let s = &PERSISTED;
    let g = s.grants.read();
    PersistedGrants {
        fs: g.fs.clone(),
        http: g.http.clone(),
    }
}

/// Lazy auto-restore: on the first `add_*_allow(...)` call after process
/// start, push every persisted entry into the live policy. Idempotent —
/// the latch flips on first run, so subsequent calls cost a single
/// atomic load. `configure(...)` flips the latch directly because it
/// already merges persisted grants into its initial allow-list.
fn auto_restore_persisted() {
    let s = &PERSISTED;
    if s.auto_restored.swap(true, Ordering::AcqRel) {
        return;
    }
    // Snapshot under the read lock, then mutate the policy without
    // holding it (avoids any chance of deadlock against the policy
    // RwLock — they're independent locks but reentry is still cheaper
    // to dodge).
    let snapshot = {
        let g = s.grants.read();
        (
            g.fs.iter().cloned().collect::<Vec<_>>(),
            g.http.iter().cloned().collect::<Vec<_>>(),
        )
    };
    // One write-lock acquisition for the whole fs scope (every
    // persisted entry), then a separate one for http. The previous
    // shape took the lock once per entry, which on a cold start with
    // a few dozen persisted grants degraded into a noticeable burst
    // of write-lock contention on the first `add_*_allow` call from
    // any thread.
    if !snapshot.0.is_empty() {
        with_policy_write(|p| {
            let mut grew = false;
            for entry in snapshot.0 {
                let absolute = expand_tilde(&entry);
                let pattern = format!("{}/**", absolute.trim_end_matches('/'));
                // Direct policy mutation — `add_fs_allow` would recurse
                // into `auto_restore_persisted()` and short-circuit, but
                // we already hold the latch, so call the lock-touching
                // code inline to keep the call graph obvious.
                if !p.fs.allow.iter().any(|existing| existing == &pattern) {
                    p.fs.allow.push(pattern);
                    grew = true;
                }
            }
            // One rebuild for the whole batch — `add_fs_allow` rebuilds per
            // call, but we're inlining a bulk insert here so amortize the
            // glob-compile cost across the entire restore.
            if grew {
                p.fs.rebuild_allow();
            }
            p.configured = true;
        });
    }
    if !snapshot.1.is_empty() {
        with_policy_write(|p| {
            let mut grew = false;
            for entry in snapshot.1 {
                if !p.http.allow.iter().any(|existing| existing == &entry) {
                    p.http.allow.push(entry);
                    grew = true;
                }
            }
            if grew {
                p.http.rebuild_allow();
            }
            p.configured = true;
        });
    }
}

/// Write the in-memory grants to disk. Best-effort: a tmp-then-rename
/// keeps a crash mid-write from leaving a half-written file the next
/// launch would silently treat as empty. Errors bubble up so the
/// dispatch arms can surface them — callers shouldn't silently lose a
/// "grant accepted" UX response when the disk write actually failed.
fn flush_persisted() -> Result<(), String> {
    let s = &PERSISTED;
    let Some(path) = s.storage_path.read().clone() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create parent: {e}"))?;
    }
    let payload = {
        let g = s.grants.read();
        serde_json::to_string(&*g).map_err(|e| format!("serialize: {e}"))?
    };
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, payload).map_err(|e| format!("write tmp: {e}"))?;
    fs::rename(&tmp, &path).map_err(|e| format!("rename: {e}"))?;
    Ok(())
}

/// Persist a path so it's auto-restored on the next launch. Adds the
/// entry to the live policy too — the persisted JSON is the durable
/// mirror of the in-memory allow-list, not a separate authorization.
/// Returns `true` if the entry is newly persisted, `false` if it was
/// already there (idempotent).
pub fn persist_grant(path: &str, kind: GrantKind) -> Result<bool, String> {
    auto_restore_persisted();
    // Always extend the live policy first — even on a duplicate-persist
    // call, the live policy might have been wiped by a `configure(...)`
    // that didn't merge persisted entries (legacy callers, tests).
    match kind {
        GrantKind::Fs => {
            let absolute = expand_tilde(path);
            let pattern = format!("{}/**", absolute.trim_end_matches('/'));
            add_fs_allow(&pattern);
        },
        GrantKind::Http => {
            add_http_allow(path);
        },
    }
    let s = &PERSISTED;
    let added = {
        let mut g = s.grants.write();
        match kind {
            GrantKind::Fs => g.fs.insert(path.to_string()),
            GrantKind::Http => g.http.insert(path.to_string()),
        }
    };
    if added {
        flush_persisted()?;
    }
    Ok(added)
}

/// Remove an entry from the persisted JSON. Does NOT touch the live
/// policy — the caller decides whether to also `removeFsAllow(...)` /
/// `removeHttpAllow(...)`. This split mirrors the original plugin
/// contract: persistence and the runtime allow-list are separate
/// concerns once the user has granted them.
pub fn revoke_persisted(path: &str, kind: GrantKind) -> Result<bool, String> {
    let s = &PERSISTED;
    let removed = {
        let mut g = s.grants.write();
        match kind {
            GrantKind::Fs => g.fs.remove(path),
            GrantKind::Http => g.http.remove(path),
        }
    };
    if removed {
        flush_persisted()?;
    }
    Ok(removed)
}

/// Snapshot of every currently-persisted entry of the requested kind.
/// Order is stable thanks to `BTreeSet` — handy for diffing.
pub fn list_persisted(kind: GrantKind) -> Vec<String> {
    let s = &PERSISTED;
    let g = s.grants.read();
    match kind {
        GrantKind::Fs => g.fs.iter().cloned().collect(),
        GrantKind::Http => g.http.iter().cloned().collect(),
    }
}

/// Membership check — no I/O, no policy mutation.
pub fn has_persisted(path: &str, kind: GrantKind) -> bool {
    let s = &PERSISTED;
    let g = s.grants.read();
    match kind {
        GrantKind::Fs => g.fs.contains(path),
        GrantKind::Http => g.http.contains(path),
    }
}

/// Drop every persisted entry of a kind. Returns success even when the
/// list was already empty — the disk write is the only failure mode and
/// it only fires on actual changes.
pub fn clear_persisted(kind: GrantKind) -> Result<(), String> {
    let s = &PERSISTED;
    let cleared = {
        let mut g = s.grants.write();
        let set = match kind {
            GrantKind::Fs => &mut g.fs,
            GrantKind::Http => &mut g.http,
        };
        let was_empty = set.is_empty();
        set.clear();
        !was_empty
    };
    if cleared {
        flush_persisted()?;
    }
    Ok(())
}

#[cfg(test)]
fn set_persisted_storage_for_tests(path: PathBuf) {
    // Re-init the persisted state with a sandbox path. Used by every
    // grants test to avoid stomping the user's real data dir.
    let s = &PERSISTED;
    *s.grants.write() = PersistedGrants::default();
    *s.storage_path.write() = Some(path);
    s.auto_restored.store(false, Ordering::Release);
}

// Cross-module test gate: anything hitting `check_fs` or `check_http`
// must hold this while security tests flip the global policy, otherwise
// fs / http tests race and see `configured: true` mid-run.
#[cfg(test)]
pub(crate) fn test_lock() -> &'static std::sync::Mutex<()> {
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    &TEST_LOCK
}

/// Test-only helper: install a `shellExecAllowedCommands` whitelist
/// without going through `configure(...)`. Lets `process` tests exercise
/// `check_shell_exec` without contaminating the rest of the policy.
#[cfg(test)]
pub(crate) fn set_shell_exec_allowed_for_tests(allowed: Vec<String>) {
    with_policy_write(|p| {
        p.shell_exec_allowed = allowed;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_literal() {
        assert!(matches("/home/a.txt", "/home/a.txt"));
        assert!(!matches("/home/a.txt", "/home/b.txt"));
    }

    #[test]
    fn glob_star_within_segment() {
        assert!(matches("/home/*.txt", "/home/a.txt"));
        assert!(!matches("/home/*.txt", "/home/sub/a.txt"));
    }

    #[test]
    fn glob_double_star_crosses_segments() {
        assert!(matches("/home/**", "/home/sub/a.txt"));
        assert!(matches("/home/**/*.log", "/home/a/b/c.log"));
    }

    #[test]
    fn deny_overrides_allow() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        POLICY.store(Arc::new(Policy {
            fs: Scope::new(
                vec!["/home/**".into()],
                vec!["/home/secret/**".into()],
            ),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        }));
        assert!(check_fs("/home/a.txt").is_ok());
        assert!(check_fs("/home/secret/k.pem").is_err());
        POLICY.store(Arc::new(Policy::default()));
    }

    #[test]
    fn default_deny_blocks_unlisted() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        POLICY.store(Arc::new(Policy {
            fs: Scope::default(),
            http: Scope::default(),
            default_deny: true,
            configured: true,
            shell_exec_allowed: Vec::new(),
        }));
        assert!(check_fs("/anywhere").is_err());
        POLICY.store(Arc::new(Policy::default()));
    }

    // -----------------------------------------------------------------
    // W4.3 — negative path ACL coverage
    //
    // Each test grabs `test_lock()` because the security policy is
    // process-global (`POLICY: LazyLock<ArcSwap<Policy>>`). All tests
    // restore `Policy::default()` on exit so a panic mid-test doesn't
    // leave `configured: true` and contaminate fs/http suites.
    // -----------------------------------------------------------------

    /// Helper: install a policy + return a guard that restores defaults
    /// when dropped. Without this, a `#[should_panic]` or mid-test
    /// `assert!` failure would leak `configured: true` into the next
    /// security/fs/http test in the same `cargo test` invocation.
    struct PolicyGuard;
    impl Drop for PolicyGuard {
        fn drop(&mut self) {
            POLICY.store(Arc::new(Policy::default()));
        }
    }
    fn install_policy(p: Policy) -> PolicyGuard {
        POLICY.store(Arc::new(p));
        PolicyGuard
    }

    #[test]
    fn fs_read_outside_allow_list_rejected() {
        // `allow: ["/home/jane/**"]` → anything else must be refused
        // even when `defaultDeny` is false (the non-empty allow list
        // implies "only these"). This is the canonical FS read negative
        // path for a deployer-configured policy.
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::new(vec!["/home/jane/**".into()], vec![]),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        // Inside allow → ok.
        assert!(check_fs("/home/jane/notes.txt").is_ok());
        // Outside → rejected with "no allow rule" message.
        let err = check_fs("/home/bob/notes.txt").unwrap_err();
        assert!(
            err.contains("no allow rule"),
            "expected 'no allow rule' message, got: {err}"
        );
    }

    #[test]
    fn fs_write_in_deny_rejected_even_inside_allow() {
        // Deny overrides allow even when the candidate is comfortably
        // inside the allow scope. Mirrors the real-world
        // `allow: home, deny: home/.ssh` deployer setup.
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::new(
                vec!["/home/**".into()],
                vec!["/home/**/.ssh/**".into()],
            ),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        assert!(check_fs("/home/jane/file.txt").is_ok());
        let err = check_fs("/home/jane/.ssh/id_rsa").unwrap_err();
        assert!(
            err.contains("denied by security policy"),
            "expected 'denied by security policy', got: {err}"
        );
    }

    #[test]
    fn default_deny_with_unlisted_path_rejected() {
        // `defaultDeny: true` + empty `allow` = closed-by-default.
        // Every call must surface the explicit `defaultDeny is on`
        // hint so the deployer knows *why* the call failed.
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::default(),
            http: Scope::default(),
            default_deny: true,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        let err = check_fs("/etc/passwd").unwrap_err();
        assert!(
            err.contains("defaultDeny is on"),
            "expected default-deny mention, got: {err}"
        );
    }

    #[test]
    fn default_deny_off_with_unlisted_path_accepted() {
        // Documents the open-by-default behaviour: empty allow + no
        // `defaultDeny` flag = "allow everything". Pinning this keeps
        // a future "secure-by-default" toggle from silently flipping
        // existing apps' policies.
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::default(),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        // Anything goes when allow is empty + defaultDeny is off.
        assert!(check_fs("/etc/passwd").is_ok());
        assert!(check_fs("/anywhere/at/all").is_ok());
    }

    #[test]
    fn glob_double_star_extension_filter() {
        // Real deployer pattern: "all .txt under /foo, and only .txt".
        // `**/*.txt` must match arbitrary depth of .txt files but
        // refuse a sibling .bin even with the same parent.
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::new(vec!["/foo/**/*.txt".into()], vec![]),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        assert!(check_fs("/foo/bar/baz.txt").is_ok());
        assert!(check_fs("/foo/a/b/c/d.txt").is_ok());
        assert!(
            check_fs("/foo/bar/baz.bin").is_err(),
            "wrong extension must reject"
        );
        assert!(
            check_fs("/foo/bar/baz.txt.bak").is_err(),
            ".txt.bak must not match *.txt"
        );
    }

    #[test]
    fn windows_alternate_data_streams_rejected_when_glob_lacks_colon() {
        // Windows ADS — `secret.txt:hidden` is a separate stream the
        // OS happily resolves. A policy that allows `*.txt` should NOT
        // implicitly cover ADS suffixes, otherwise an attacker writes
        // `notes.txt:exfil` and bypasses the file-extension allowlist.
        // Skip on non-Windows where ADS is just a literal `:` in the
        // filename and the policy semantics differ.
        if !cfg!(target_os = "windows") {
            return;
        }
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::new(vec!["c:/users/jane/*.txt".into()], vec![]),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        // Plain .txt under the allowed dir → ok.
        assert!(check_fs("C:/Users/Jane/notes.txt").is_ok());
        // ADS suffix must NOT match — the trailing `:stream` keeps the
        // path from globbing as `*.txt`. Documents the
        // "glob-without-colon-doesn't-cover-ADS" contract.
        assert!(
            check_fs("C:/Users/Jane/notes.txt:secret").is_err(),
            "ADS suffix must not match a glob that doesn't carry ':'"
        );
    }

    #[test]
    fn windows_short_names_documented_behaviour() {
        // Documents (not endorses) current Windows 8.3 short-name
        // behaviour: `SECRET~1.TXT` is matched purely by the glob's
        // text — `canonicalize_fs` only lowercases, it doesn't expand
        // short names to their long form. Deployers writing
        // `c:/foo/secret.txt` in `deny` will NOT block a caller
        // requesting `c:/foo/secret~1.txt` unless they also list the
        // short variant. Surfacing this explicitly so a future
        // canonicalize change has to update this test deliberately.
        if !cfg!(target_os = "windows") {
            return;
        }
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy {
            fs: Scope::new(
                vec!["c:/users/**".into()],
                vec!["c:/users/**/secret.txt".into()],
            ),
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        });
        // Long form blocked (deny rule wins).
        assert!(check_fs("C:/Users/Jane/secret.txt").is_err());
        // Short-name form NOT blocked — current behaviour, intentional
        // (we don't query the FS to expand). If this changes, the
        // deny rule should grow a `secret~1.txt` companion.
        assert!(check_fs("C:/Users/Jane/SECRET~1.TXT").is_ok());
    }

    // -----------------------------------------------------------------
    // W2.2 — live policy mutation (`add_fs_allow` / `add_http_allow`)
    //
    // Each helper has to be (a) idempotent — duplicate calls return
    // `false` rather than growing the allow-list, (b) atomic — the
    // `check_fs` hot path must never see a partial write, and (c) the
    // `remove_*` twin must surface "wasn't there" as a return value
    // so the persisted-scope plugin can stay in sync with what the
    // user actually granted.
    // -----------------------------------------------------------------

    #[test]
    fn add_fs_allow_is_idempotent_and_observed_by_check_fs() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        // First add returns `true` (newly added). Calling `add` flips the
        // policy from "unconfigured" → "configured" too, so a path
        // OUTSIDE the allow-list now rejects.
        assert!(add_fs_allow("/runtime/granted/**"));
        assert!(check_fs("/runtime/granted/file.txt").is_ok());
        assert!(
            check_fs("/somewhere/else").is_err(),
            "policy must enforce after first add (configured=true)"
        );

        // Second add of the same entry returns `false` (idempotent) and
        // the allow-list MUST NOT grow.
        assert!(!add_fs_allow("/runtime/granted/**"));
        let p = POLICY.load();
        assert_eq!(
            p.fs.allow.iter().filter(|x| *x == "/runtime/granted/**").count(),
            1,
            "duplicate add must not grow allow list"
        );
    }

    #[test]
    fn remove_fs_allow_returns_false_when_absent() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        // Nothing added yet → remove is a no-op that returns `false`.
        assert!(!remove_fs_allow("/never/added"));
        // Round-trip: add, then remove twice — first remove succeeds,
        // second is a no-op.
        assert!(add_fs_allow("/granted/**"));
        assert!(remove_fs_allow("/granted/**"));
        assert!(!remove_fs_allow("/granted/**"));
    }

    #[test]
    fn add_http_allow_does_not_touch_fs_scope() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        // Cross-scope contamination is the primary regression risk —
        // `add_http_allow` writing into `POLICY.fs` would silently
        // grant FS access on every HTTP grant. Pin the boundary.
        assert!(add_http_allow("https://api.example.com/**"));
        let p = POLICY.load();
        assert_eq!(p.http.allow.len(), 1);
        assert!(p.fs.allow.is_empty(), "fs scope must stay untouched");
    }

    #[test]
    fn dispatch_routes_add_fs_allow_to_helper() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        // Round-trip via the IPC dispatch arm so the persisted-scope
        // plugin's `cx.os_call("security", "addFsAllow", ...)` path
        // stays covered. First call → true; second → false.
        let r1 = dispatch(
            "addFsAllow",
            &serde_json::json!({ "path": "/dispatch/granted" }),
        )
        .expect("addFsAllow");
        assert_eq!(r1, Value::Bool(true));
        let r2 = dispatch(
            "addFsAllow",
            &serde_json::json!({ "path": "/dispatch/granted" }),
        )
        .expect("addFsAllow idempotent");
        assert_eq!(r2, Value::Bool(false));

        // Missing 'path' surfaces the same actionable error every
        // other dispatch arm uses.
        let err = dispatch("addFsAllow", &serde_json::json!({})).unwrap_err();
        assert!(err.contains("missing 'path'"), "got: {err}");
    }

    #[test]
    fn canonicalize_fs_lowercase_consistent_on_windows() {
        // `canonicalize_fs` must produce deterministic output regardless
        // of the input case on Windows, otherwise two policy lookups for
        // semantically-equivalent paths can diverge. Pin both the
        // separator normalization (`\\` → `/`) and the casing.
        let upper = canonicalize_fs("C:\\Users\\Jane\\Notes.TXT");
        let lower = canonicalize_fs("c:/users/jane/notes.txt");
        if cfg!(target_os = "windows") {
            assert_eq!(upper, lower);
            assert_eq!(upper, "c:/users/jane/notes.txt");
        } else {
            // On non-Windows, casing is preserved. Separators still
            // normalize because we replace `\\` unconditionally.
            assert_eq!(canonicalize_fs("/Home/Jane"), "/Home/Jane");
        }
    }

    // -----------------------------------------------------------------
    // Persisted grants — folded from `@tynd/plugin-persisted-scope`
    //
    // Every test points the persisted state at a sandbox JSON via
    // `set_persisted_storage_for_tests` so the user's real
    // `<data_dir>/.../security-persisted-grants.json` stays untouched.
    // We also grab `test_lock()` because `persist_grant` mutates the
    // process-global policy.
    // -----------------------------------------------------------------

    fn temp_persisted(label: &str) -> PathBuf {
        // Mix in pid + nanos to keep parallel test threads from
        // colliding on the same sandbox file.
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut p = std::env::temp_dir();
        p.push(format!(
            "tynd-security-persisted-{}-{}-{label}.json",
            std::process::id(),
            stamp
        ));
        let _ = fs::remove_file(&p);
        p
    }

    #[test]
    fn persist_grant_round_trips_through_disk() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("roundtrip");
        set_persisted_storage_for_tests(file.clone());

        assert!(persist_grant("~/Documents", GrantKind::Fs).unwrap());
        assert!(persist_grant("https://api.example.com", GrantKind::Http).unwrap());

        // File now exists + parses back to the same set.
        let raw = fs::read_to_string(&file).expect("file written");
        let parsed: PersistedGrants = serde_json::from_str(&raw).unwrap();
        assert!(parsed.fs.contains("~/Documents"));
        assert!(parsed.http.contains("https://api.example.com"));

        // List APIs reflect the in-memory mirror.
        assert_eq!(list_persisted(GrantKind::Fs), vec!["~/Documents"]);
        assert_eq!(
            list_persisted(GrantKind::Http),
            vec!["https://api.example.com"]
        );
        assert!(has_persisted("~/Documents", GrantKind::Fs));
        assert!(!has_persisted("~/Other", GrantKind::Fs));

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn persist_grant_is_idempotent() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("idem");
        set_persisted_storage_for_tests(file.clone());

        assert!(persist_grant("/tmp/foo", GrantKind::Fs).unwrap());
        // Second call returns false (no-op on the persisted set) but
        // still keeps the file consistent.
        assert!(!persist_grant("/tmp/foo", GrantKind::Fs).unwrap());
        assert_eq!(list_persisted(GrantKind::Fs).len(), 1);

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn persist_grant_extends_live_policy_with_glob_suffix() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("live-policy");
        set_persisted_storage_for_tests(file.clone());

        persist_grant("/granted/dir", GrantKind::Fs).unwrap();
        // The persisted entry hits the live `fs.allow` as `/granted/dir/**`
        // so an FS call inside the granted subtree passes the policy
        // check that just got flipped on (configured = true).
        assert!(check_fs("/granted/dir/file.txt").is_ok());
        assert!(check_fs("/elsewhere/file.txt").is_err());

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn revoke_persisted_does_not_touch_live_policy() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("revoke-live-untouched");
        set_persisted_storage_for_tests(file.clone());

        persist_grant("/granted/dir", GrantKind::Fs).unwrap();
        // Revoking the persisted entry must NOT remove `/granted/dir/**`
        // from the live policy — that's a separate decision the caller
        // makes via `removeFsAllow(...)`. Mirrors the original plugin
        // contract documented on `revoke_persisted`.
        assert!(revoke_persisted("/granted/dir", GrantKind::Fs).unwrap());
        assert!(check_fs("/granted/dir/file.txt").is_ok());
        assert!(!has_persisted("/granted/dir", GrantKind::Fs));

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn clear_persisted_only_drops_target_kind() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("clear-kind");
        set_persisted_storage_for_tests(file.clone());

        persist_grant("/data", GrantKind::Fs).unwrap();
        persist_grant("https://x", GrantKind::Http).unwrap();
        clear_persisted(GrantKind::Fs).unwrap();
        assert!(list_persisted(GrantKind::Fs).is_empty());
        assert_eq!(list_persisted(GrantKind::Http), vec!["https://x"]);

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn dispatch_routes_persist_grant() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("dispatch");
        set_persisted_storage_for_tests(file.clone());

        let r = dispatch(
            "persistGrant",
            &serde_json::json!({ "path": "/from/dispatch", "kind": "fs" }),
        )
        .expect("persistGrant");
        assert_eq!(r, Value::Bool(true));
        let listed = dispatch("listPersisted", &serde_json::json!({ "kind": "fs" }))
            .expect("listPersisted");
        assert_eq!(listed, serde_json::json!(["/from/dispatch"]));
        let has = dispatch(
            "hasPersisted",
            &serde_json::json!({ "path": "/from/dispatch", "kind": "fs" }),
        )
        .expect("hasPersisted");
        assert_eq!(has, Value::Bool(true));

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn dispatch_persist_grant_invalid_kind_errors() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _p = install_policy(Policy::default());
        let file = temp_persisted("bad-kind");
        set_persisted_storage_for_tests(file.clone());

        let err = dispatch(
            "persistGrant",
            &serde_json::json!({ "path": "/x", "kind": "filesystem" }),
        )
        .unwrap_err();
        assert!(err.contains("kind"), "got: {err}");

        let _ = fs::remove_file(&file);
    }

    #[test]
    fn expand_tilde_resolves_home_when_available() {
        if let Some(home) = dirs::home_dir() {
            let expanded = expand_tilde("~/foo");
            assert!(
                expanded.starts_with(&home.to_string_lossy().into_owned()),
                "expected expansion under {home:?}, got {expanded}"
            );
            assert!(expanded.ends_with("foo"));
        }
    }

    #[test]
    fn expand_tilde_passes_absolute_paths_through() {
        assert_eq!(expand_tilde("/tmp/foo"), "/tmp/foo");
        assert_eq!(expand_tilde("relative/foo"), "relative/foo");
    }
}
