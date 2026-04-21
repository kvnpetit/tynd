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

use parking_lot::RwLock;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Default, Deserialize)]
pub struct PolicyInput {
    #[serde(default)]
    pub fs: ScopeInput,
    #[serde(default)]
    pub http: ScopeInput,
    /// When true, an empty `allow` list denies every call of that scope.
    /// Default is false (open-by-default).
    #[serde(default, rename = "defaultDeny")]
    pub default_deny: bool,
}

#[derive(Debug, Default, Deserialize)]
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

/// Tiny glob: `**` = any sequence (incl. `/`), `*` = any sequence without
/// `/`, everything else literal. Case folded to match `canonicalize_fs`.
fn matches(pattern: &str, input: &str) -> bool {
    let pat = if cfg!(target_os = "windows") {
        pattern.replace('\\', "/").to_lowercase()
    } else {
        pattern.to_string()
    };
    glob_match(&pat, input)
}

fn glob_match(pattern: &str, input: &str) -> bool {
    // Translate to a simple state machine without regex deps.
    let pat: Vec<char> = pattern.chars().collect();
    let inp: Vec<char> = input.chars().collect();
    matcher(&pat, 0, &inp, 0, 0)
}

/// Hard cap on recursion. Patterns with many `*` segments can otherwise
/// grow combinatorially — a crafted user-supplied policy like
/// `**/**/**/…` against a long path would blow the stack. Returning false
/// past the cap is the safe fallback (denied by allow list, ignored by
/// deny list; caller can widen the allow rule if needed).
const GLOB_MAX_DEPTH: u32 = 64;

fn matcher(pat: &[char], pi: usize, inp: &[char], ii: usize, depth: u32) -> bool {
    if depth > GLOB_MAX_DEPTH {
        return false;
    }
    if pi == pat.len() {
        return ii == inp.len();
    }
    if pat[pi] == '*' {
        let cross_sep = pi + 1 < pat.len() && pat[pi + 1] == '*';
        let next_pi = if cross_sep { pi + 2 } else { pi + 1 };
        // Try every split of the remaining input.
        for i in ii..=inp.len() {
            if !cross_sep && i > ii && inp[i - 1] == '/' {
                break;
            }
            if matcher(pat, next_pi, inp, i, depth + 1) {
                return true;
            }
        }
        return false;
    }
    if ii < inp.len() && pat[pi] == inp[ii] {
        return matcher(pat, pi + 1, inp, ii + 1, depth + 1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Tests mutate the global policy; serialize around a dedicated gate so
    // they don't stomp each other under `cargo test`'s parallel runner.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn glob_literal() {
        assert!(glob_match("/home/a.txt", "/home/a.txt"));
        assert!(!glob_match("/home/a.txt", "/home/b.txt"));
    }

    #[test]
    fn glob_star_within_segment() {
        assert!(glob_match("/home/*.txt", "/home/a.txt"));
        assert!(!glob_match("/home/*.txt", "/home/sub/a.txt"));
    }

    #[test]
    fn glob_double_star_crosses_segments() {
        assert!(glob_match("/home/**", "/home/sub/a.txt"));
        assert!(glob_match("/home/**/*.log", "/home/a/b/c.log"));
    }

    #[test]
    fn deny_overrides_allow() {
        let _g = TEST_LOCK
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
        };
        assert!(check_fs("/home/a.txt").is_ok());
        assert!(check_fs("/home/secret/k.pem").is_err());
        *policy().write() = Policy::default();
    }

    #[test]
    fn default_deny_blocks_unlisted() {
        let _g = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *policy().write() = Policy {
            fs: Scope::default(),
            http: Scope::default(),
            default_deny: true,
            configured: true,
        };
        assert!(check_fs("/anywhere").is_err());
        *policy().write() = Policy::default();
    }
}
