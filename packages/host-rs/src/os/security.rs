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

use std::{path::Path, sync::OnceLock};

use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::Value;
use specta::Type;

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

#[derive(Debug, Default)]
struct Scope {
    allow: Vec<String>,
    deny: Vec<String>,
}

#[derive(Debug, Default)]
struct Policy {
    fs: Scope,
    http: Scope,
    default_deny: bool,
    configured: bool,
    /// Whitelist for `process.execShell` (see `PolicyInput::shell_exec_allowed_commands`).
    shell_exec_allowed: Vec<String>,
}

static POLICY: OnceLock<RwLock<Policy>> = OnceLock::new();

fn policy() -> &'static RwLock<Policy> {
    POLICY.get_or_init(|| RwLock::new(Policy::default()))
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "configure" => configure(args),
        "isFsAllowed" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "security.isFsAllowed: missing 'path'".to_string())?;
            Ok(Value::Bool(check_fs(path).is_ok()))
        },
        "isHttpAllowed" => {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "security.isHttpAllowed: missing 'url'".to_string())?;
            Ok(Value::Bool(check_http(url).is_ok()))
        },
        _ => Err(format!("security.{method}: unknown method")),
    }
}

fn configure(args: &Value) -> Result<Value, String> {
    let input: PolicyInput = serde_json::from_value(args.clone())
        .map_err(|e| format!("security.configure: invalid shape — {e}"))?;
    let next = Policy {
        fs: Scope {
            allow: input.fs.allow,
            deny: input.fs.deny,
        },
        http: Scope {
            allow: input.http.allow,
            deny: input.http.deny,
        },
        default_deny: input.default_deny,
        configured: true,
        shell_exec_allowed: input.shell_exec_allowed_commands,
    };
    *policy().write() = next;
    Ok(Value::Null)
}

pub fn check_fs(path: &str) -> Result<(), String> {
    let p = policy().read();
    if !p.configured {
        return Ok(());
    }
    let canonical = canonicalize_fs(path);
    check(&canonical, &p.fs, p.default_deny, "fs")
}

pub fn check_http(url: &str) -> Result<(), String> {
    let p = policy().read();
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
    let p = policy().read();
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
    if scope.deny.iter().any(|pat| matches(pat, candidate)) {
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
    if scope.allow.iter().any(|pat| matches(pat, candidate)) {
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

/// Glob match via the `globset` crate. The pattern's separator is
/// normalized the same way `canonicalize_fs` normalizes the input on
/// Windows (`\\` → `/` + lowercase) so a rule written as
/// `C:\Users\*\Desktop` still matches `c:/users/jane/desktop`. The old
/// hand-rolled matcher only normalized the *input*, leaving Windows
/// patterns asymmetric — that's fixed here.
///
/// `globset` compiles patterns into a regex DFA and short-circuits on
/// the first mismatch, so it doesn't need the recursion cap the
/// previous implementation carried (which silently denied legitimate
/// matches when policies had many `**` segments).
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
    policy().write().shell_exec_allowed = allowed;
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
        *policy().write() = Policy {
            fs: Scope {
                allow: vec!["/home/**".into()],
                deny: vec!["/home/secret/**".into()],
            },
            http: Scope::default(),
            default_deny: false,
            configured: true,
            shell_exec_allowed: Vec::new(),
        };
        assert!(check_fs("/home/a.txt").is_ok());
        assert!(check_fs("/home/secret/k.pem").is_err());
        *policy().write() = Policy::default();
    }

    #[test]
    fn default_deny_blocks_unlisted() {
        let _g = test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *policy().write() = Policy {
            fs: Scope::default(),
            http: Scope::default(),
            default_deny: true,
            configured: true,
            shell_exec_allowed: Vec::new(),
        };
        assert!(check_fs("/anywhere").is_err());
        *policy().write() = Policy::default();
    }

    // -----------------------------------------------------------------
    // W4.3 — negative path ACL coverage
    //
    // Each test grabs `test_lock()` because the security policy is
    // process-global (`POLICY: OnceLock<RwLock<Policy>>`). All tests
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
            *policy().write() = Policy::default();
        }
    }
    fn install_policy(p: Policy) -> PolicyGuard {
        *policy().write() = p;
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
            fs: Scope {
                allow: vec!["/home/jane/**".into()],
                deny: vec![],
            },
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
            fs: Scope {
                allow: vec!["/home/**".into()],
                deny: vec!["/home/**/.ssh/**".into()],
            },
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
            fs: Scope {
                allow: vec!["/foo/**/*.txt".into()],
                deny: vec![],
            },
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
            fs: Scope {
                allow: vec!["c:/users/jane/*.txt".into()],
                deny: vec![],
            },
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
            fs: Scope {
                allow: vec!["c:/users/**".into()],
                deny: vec!["c:/users/**/secret.txt".into()],
            },
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
}
