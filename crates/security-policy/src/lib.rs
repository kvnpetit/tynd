//! Pure security policy engine for Tynd host APIs.
//!
//! This crate deliberately avoids host dispatch, event emission, and storage
//! side effects. `tynd-host` owns those effects and delegates policy decisions
//! here.

#![forbid(unsafe_code)]

use globset::{GlobSet, GlobSetBuilder};
use serde::Deserialize;
use specta::Type;

mod grants;

pub use grants::{
    validate_url_pattern, FsAccess, Grants, GrantsSnapshot, PathGrant, MAX_PATH_GRANTS,
    MAX_URL_GRANTS,
};

#[derive(Debug, Default, Deserialize, Type)]
pub struct PolicyInput {
    #[serde(default)]
    pub fs: ScopeInput,
    #[serde(default)]
    pub http: ScopeInput,
    #[serde(default, rename = "defaultDeny")]
    pub default_deny: bool,
    #[serde(default, rename = "shellExecAllowedCommands")]
    pub shell_exec_allowed_commands: Vec<String>,
    #[serde(default, rename = "processAllowedBinaries")]
    pub process_allowed_binaries: Option<Vec<String>>,
    #[serde(default, rename = "allowedSensitiveCapabilities")]
    pub allowed_sensitive_capabilities: Vec<String>,
    /// Keep runtime scope grants across restarts (stored in the app data
    /// directory and re-validated on load).
    #[serde(default, rename = "persistGrants")]
    pub persist_grants: bool,
}

#[derive(Debug, Default, Deserialize, Type)]
pub struct ScopeInput {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// How the patterns of a [`Scope`] are normalized before compiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatternKind {
    /// Filesystem paths: separators unified and, on Windows, lowercased.
    Fs,
    /// URLs: scheme and authority lowercased, default port dropped. The path
    /// is matched as-is (never case-folded).
    Http,
}

#[derive(Debug, Default, Clone)]
pub struct Scope {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    allow_set: GlobSet,
    deny_set: GlobSet,
}

impl Scope {
    /// Filesystem scope.
    pub fn new(allow: Vec<String>, deny: Vec<String>) -> Self {
        Self::build_lenient(PatternKind::Fs, allow, deny)
    }

    /// Filesystem scope; rejects invalid globs.
    pub fn try_new(allow: Vec<String>, deny: Vec<String>) -> Result<Self, String> {
        Self::build_strict(PatternKind::Fs, allow, deny)
    }

    /// HTTP scope: patterns are normalized as URLs, candidates are parsed and
    /// canonicalized by [`SecurityPolicy::check_http`].
    pub fn new_http(allow: Vec<String>, deny: Vec<String>) -> Self {
        Self::build_lenient(PatternKind::Http, allow, deny)
    }

    /// HTTP scope; rejects invalid globs.
    pub fn try_new_http(allow: Vec<String>, deny: Vec<String>) -> Result<Self, String> {
        Self::build_strict(PatternKind::Http, allow, deny)
    }

    fn build_lenient(kind: PatternKind, allow: Vec<String>, deny: Vec<String>) -> Self {
        let allow_set = compile_kind_lenient(kind, &allow, false);
        let deny_set = compile_kind_lenient(kind, &deny, deny_case_insensitive(kind));
        Self {
            allow,
            deny,
            allow_set,
            deny_set,
        }
    }

    fn build_strict(
        kind: PatternKind,
        allow: Vec<String>,
        deny: Vec<String>,
    ) -> Result<Self, String> {
        let allow_set = try_compile_kind(kind, &allow, false)?;
        let deny_set = try_compile_kind(kind, &deny, deny_case_insensitive(kind))?;
        Ok(Self {
            allow,
            deny,
            allow_set,
            deny_set,
        })
    }

    /// Deny-by-default filesystem check for an allow-list that must name every
    /// reachable path (the `asset://` scope). `path` must already be
    /// kernel-resolved by the caller; an empty allow list denies everything.
    pub fn check_fs_allow_list(&self, path: &str, kind: &str) -> Result<(), String> {
        self.check(&canonicalize_fs(path), true, kind)
    }

    fn check(&self, candidate: &str, default_deny: bool, kind: &str) -> Result<(), String> {
        self.check_any(&[candidate], default_deny, kind)
    }

    /// Whether a deny rule matches `path` (canonicalized as a filesystem
    /// path first). The allow rules play no part.
    #[must_use]
    pub fn denies_fs(&self, path: &str) -> bool {
        self.deny_set.is_match(canonicalize_fs(path))
    }

    /// `candidates[0]` is the primary (reported) form. A deny rule matching
    /// any candidate denies; an allow rule matching any candidate allows.
    fn check_any(&self, candidates: &[&str], default_deny: bool, kind: &str) -> Result<(), String> {
        let primary = candidates.first().copied().unwrap_or("");
        if candidates.iter().any(|c| self.deny_set.is_match(c)) {
            return Err(format!("{kind}: denied by security policy - '{primary}'"));
        }
        if self.allow.is_empty() {
            if default_deny {
                return Err(format!(
                    "{kind}: no allow rule matches '{primary}' and defaultDeny is on"
                ));
            }
            return Ok(());
        }
        if candidates.iter().any(|c| self.allow_set.is_match(c)) {
            Ok(())
        } else {
            Err(format!("{kind}: no allow rule matches '{primary}'"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    pub fs: Scope,
    pub http: Scope,
    pub default_deny: bool,
    pub configured: bool,
    pub shell_exec_allowed: Vec<String>,
    pub process_allowed_binaries: Option<Vec<String>>,
    pub allowed_sensitive_capabilities: Vec<String>,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            fs: Scope::default(),
            http: Scope::new_http(Vec::new(), Vec::new()),
            default_deny: false,
            configured: false,
            shell_exec_allowed: Vec::new(),
            process_allowed_binaries: None,
            allowed_sensitive_capabilities: Vec::new(),
        }
    }
}

impl SecurityPolicy {
    pub fn from_input(
        input: PolicyInput,
        fs_allow: Vec<String>,
        http_allow: Vec<String>,
    ) -> Result<Self, String> {
        for rule in &fs_allow {
            if let Some(warning) = fs_allow_rule_warning(rule) {
                tracing::warn!(target: "tynd::security", "{warning}");
            }
        }
        Ok(Self {
            fs: Scope::try_new(fs_allow, input.fs.deny)?,
            http: Scope::try_new_http(http_allow, input.http.deny)?,
            default_deny: input.default_deny,
            configured: true,
            shell_exec_allowed: input.shell_exec_allowed_commands,
            process_allowed_binaries: input.process_allowed_binaries,
            allowed_sensitive_capabilities: input.allowed_sensitive_capabilities,
        })
    }

    pub fn check_fs(&self, path: &str) -> Result<(), String> {
        self.check_fs_resolved(path)
    }

    /// Evaluate a filesystem path that the host has already resolved through
    /// the kernel and confined to its capability roots.
    ///
    /// This crate remains a pure matcher and cannot make a lexical path safe
    /// on its own. Host/plugin boundaries must resolve existing paths (or the
    /// deepest existing ancestor for creation) before calling this method.
    pub fn check_fs_resolved(&self, path: &str) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }
        let canonical = canonicalize_fs(path);
        self.fs.check(&canonical, self.default_deny, "fs")
    }

    /// Only the `fs.deny` rules: a resolved path another allow-list admitted
    /// (the `asset://` scope) still cannot reach what the fs policy denies.
    pub fn check_fs_not_denied(&self, path: &str) -> Result<(), String> {
        let canonical = canonicalize_fs(path);
        if self.fs.deny_set.is_match(&canonical) {
            return Err(format!("fs: denied by security policy - '{canonical}'"));
        }
        Ok(())
    }

    /// Whether an explicit `fs.allow` rule anchored at an absolute directory
    /// names `path`, and `path` sits under that directory. Unlike
    /// [`Self::check_fs_resolved`], an empty allow list admits nothing: only
    /// a rule the deployer wrote can reach past the host's capability roots.
    /// A rule without a fixed absolute leading directory (`**/*.pdf`) never
    /// matches here: it only filters inside the roots and the grants. Deny
    /// rules are not consulted here.
    #[must_use]
    pub fn fs_allow_rule_matches(&self, path: &str) -> bool {
        self.fs_allow_anchor(path).is_some()
    }

    /// The directory an `fs.allow` rule matching `path` is anchored at: the
    /// longest wildcard-free leading directory of the rule (the parent, for a
    /// rule naming a single path). Among several matching rules the deepest
    /// anchor wins. `None` when no rule matches or the matching rules start
    /// with a wildcard (`**/*.txt`), which only filters inside the roots.
    #[must_use]
    pub fn fs_allow_anchor(&self, path: &str) -> Option<String> {
        if !self.configured {
            return None;
        }
        let canonical = canonicalize_fs(path);
        self.fs
            .allow_set
            .matches(&canonical)
            .into_iter()
            .filter_map(|index| self.fs.allow.get(index))
            .filter_map(|rule| literal_anchor(rule))
            .filter(|anchor| is_under(&canonical, &canonicalize_fs(anchor)))
            .max_by_key(String::len)
    }

    pub fn check_fs_configured(&self, path: &str) -> Result<(), String> {
        self.check_fs_resolved_configured(path)
    }

    pub fn check_fs_resolved_configured(&self, path: &str) -> Result<(), String> {
        if !self.configured {
            return Err("fs: security policy is not configured".into());
        }
        let canonical = canonicalize_fs(path);
        self.fs.check(&canonical, self.default_deny, "fs")
    }

    pub fn check_http(&self, url: &str) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }
        self.check_http_inner(url)
    }

    fn check_http_inner(&self, url: &str) -> Result<(), String> {
        let (full, path_only) = http_candidates(url)?;
        // An encoded separator lets `*` match across segments (`..%2F`) and may
        // be decoded by the server into a traversal. Refuse unless the allow
        // rule that matches the candidate itself spells out every such
        // encoding the candidate contains (deny rules still apply as usual).
        let ambiguous = path_has_encoded_separator_or_dot_segment(candidate_path(&path_only));
        self.http
            .check_any(&[&full, &path_only], self.default_deny, "http")?;
        if ambiguous {
            let candidate = candidate_path(&path_only).to_ascii_lowercase();
            let markers = ambiguous_markers(&candidate);
            let spelling: Vec<String> = self
                .http
                .allow
                .iter()
                .filter(|rule| {
                    let rule = normalize_http_pattern(rule).to_ascii_lowercase();
                    markers
                        .iter()
                        .all(|needle| rule_spells_marker(&rule, needle))
                })
                .cloned()
                .collect();
            let opted_in = !markers.is_empty() && {
                let scope = Scope::new_http(spelling, Vec::new());
                [full.as_str(), path_only.as_str()]
                    .iter()
                    .any(|c| scope.allow_set.is_match(c))
            };
            if !opted_in {
                return Err(format!(
                    "http: encoded path separator or dot segment is not allowed - '{url}'"
                ));
            }
        }
        Ok(())
    }

    pub fn check_http_configured(&self, url: &str) -> Result<(), String> {
        if !self.configured {
            return Err("http: security policy is not configured".into());
        }
        self.check_http_inner(url)
    }

    pub fn check_sensitive_capability(&self, capability: &str) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }
        if self
            .allowed_sensitive_capabilities
            .iter()
            .any(|allowed| allowed == capability)
        {
            return Ok(());
        }
        Err(format!(
            "security: sensitive capability '{capability}' is not allowed; set \
             `app.start({{ security: {{ allowedSensitiveCapabilities: [...] }} }})` to opt in"
        ))
    }

    pub fn check_process(&self, binary: &str) -> Result<(), String> {
        let allowed = self
            .process_allowed_binaries
            .as_ref()
            .filter(|_| self.configured)
            .ok_or_else(|| {
                "process: not permitted - set `app.start({ security: { \
                 processAllowedBinaries: [...] } })` to opt in"
                    .to_string()
            })?;
        let canonical = canonicalize_process_binary(binary);
        if allowed
            .iter()
            .any(|entry| canonicalize_process_binary(entry) == canonical || entry == binary)
        {
            return Ok(());
        }
        Err(format!(
            "process: '{binary}' is not in `processAllowedBinaries`"
        ))
    }

    /// Shell-mode variant of [`Self::check_shell_exec`] that also refuses any
    /// caller-supplied environment variable. The shell expands `%VAR%` (cmd.exe)
    /// before parsing operators, so a caller-controlled value can smuggle
    /// commands past the first-token allowlist. `env_keys` are the keys the
    /// caller wants to set; any key refuses the call.
    pub fn check_shell_exec_with_env<'a>(
        &self,
        cmd_line: &str,
        env_keys: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), String> {
        if let Some(key) = env_keys.into_iter().next() {
            return Err(format!(
                "process.shellExec: caller-supplied env ('{key}') is not allowed in shell mode - \
                 use `process.exec` with separate `args` and `env`"
            ));
        }
        self.check_shell_exec(cmd_line)
    }

    pub fn check_shell_exec(&self, cmd_line: &str) -> Result<(), String> {
        if self.shell_exec_allowed.is_empty() {
            return Err(
                "process.shellExec: not permitted - set `app.start({ security: { \
                 shellExecAllowedCommands: [...] } })` to opt in"
                    .into(),
            );
        }
        let trimmed = cmd_line.trim_start();
        let first_token = trimmed
            .split(|c: char| c.is_whitespace() || SHELL_METACHARS.contains(&c))
            .next()
            .unwrap_or("");
        if first_token.is_empty() {
            return Err("process.shellExec: empty command".into());
        }
        if cmd_line.chars().any(|c| SHELL_METACHARS.contains(&c)) {
            return Err(
                "process.shellExec: shell metacharacters are not allowed - use `process.exec` \
                 with separate `args` to compose arguments"
                    .into(),
            );
        }
        if !self
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
}

pub fn compile_glob_set(patterns: &[String]) -> GlobSet {
    compile_kind_lenient(PatternKind::Fs, patterns, false)
}

fn compile_kind_lenient(kind: PatternKind, patterns: &[String], case_insensitive: bool) -> GlobSet {
    try_compile_kind(kind, patterns, case_insensitive).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "security: invalid glob set; treating as empty");
        GlobSet::empty()
    })
}

/// Deny rules ignore case where the default filesystem does: Windows (NTFS)
/// and macOS (APFS / HFS+), so `~/.SSH` cannot slip past a `~/.ssh` rule.
/// HTTP deny rules always ignore case.
fn deny_case_insensitive(kind: PatternKind) -> bool {
    match kind {
        PatternKind::Fs => cfg!(any(target_os = "windows", target_os = "macos")),
        PatternKind::Http => true,
    }
}

/// Case folding applied to Windows paths and path patterns. Every pattern and
/// candidate goes through this one helper (Unicode-aware) so both sides of a
/// match always fold identically.
fn fold_path_case(s: &str) -> String {
    s.to_lowercase()
}

fn normalize_pattern(kind: PatternKind, pat: &str) -> String {
    match kind {
        PatternKind::Fs => {
            if cfg!(target_os = "windows") {
                fold_path_case(&pat.replace('\\', "/"))
            } else {
                pat.to_string()
            }
        },
        PatternKind::Http => normalize_http_pattern(pat),
    }
}

fn try_compile_kind(
    kind: PatternKind,
    patterns: &[String],
    case_insensitive: bool,
) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let normalized = normalize_pattern(kind, pat);
        let glob = globset::GlobBuilder::new(&normalized)
            .literal_separator(true)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|err| format!("invalid security glob pattern '{pat}': {err}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|err| format!("failed to build security glob set: {err}"))
}

pub fn validate_glob_pattern(pattern: &str) -> Result<(), String> {
    try_compile_kind(PatternKind::Fs, &[pattern.to_string()], false).map(|_| ())
}

/// Wildcard-free leading directory of an absolute filesystem glob, in the
/// rule's own spelling (forward slashes). A rule without metacharacters
/// names one path; its anchor is the parent directory.
fn literal_anchor(rule: &str) -> Option<String> {
    let unified = if cfg!(target_os = "windows") {
        rule.replace('\\', "/")
    } else {
        rule.to_string()
    };
    let is_meta = |segment: &str| {
        segment.contains(['*', '?', '[', ']', '{', '}'])
            || (!cfg!(target_os = "windows") && segment.contains('\\'))
    };
    let segments: Vec<&str> = unified.split('/').collect();
    let literal = segments
        .iter()
        .position(|segment| is_meta(segment))
        .unwrap_or_else(|| segments.len().saturating_sub(1));
    let anchor = segments.get(..literal)?.join("/");
    let anchor = if anchor.is_empty() && unified.starts_with('/') {
        "/".to_string()
    } else if cfg!(target_os = "windows") && anchor.len() == 2 && anchor.ends_with(':') {
        format!("{anchor}/")
    } else {
        anchor
    };
    std::path::Path::new(&anchor)
        .is_absolute()
        .then_some(anchor)
}

/// `path` equals `dir` or lies under it (both canonical, forward slashes).
fn is_under(path: &str, dir: &str) -> bool {
    path == dir
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| dir.ends_with('/') || rest.starts_with('/'))
}

/// Why an `fs.allow` rule cannot reach outside the capability roots, if it
/// cannot. Only a rule whose wildcard-free leading part is an absolute
/// directory extends the scope; any other rule only filters the paths the
/// default scope or a runtime grant already admits.
#[must_use]
pub fn fs_allow_rule_warning(rule: &str) -> Option<String> {
    if literal_anchor(rule).is_some() {
        return None;
    }
    let unified = rule.replace('\\', "/");
    let wildcard_led = unified
        .split('/')
        .next()
        .is_some_and(|first| first.contains(['*', '?', '[', '{']));
    Some(if wildcard_led {
        format!(
            "security: fs.allow rule '{rule}' has no absolute leading directory: it only filters \
             paths inside the app's own directories and runtime grants, and reaches nothing \
             outside them"
        )
    } else {
        format!(
            "security: fs.allow rule '{rule}' is relative and matches no path (paths are matched \
             in absolute form); start it with an absolute directory"
        )
    })
}

/// Escape a literal path so a glob matches exactly that path.
#[must_use]
pub fn literal_glob(path: &str) -> String {
    globset::escape(path.trim_end_matches('/'))
}

/// Convert a literal directory path to a recursive glob without interpreting
/// metacharacters in the directory name.
pub fn literal_subtree_glob(path: &str) -> String {
    format!("{}/**", globset::escape(path.trim_end_matches('/')))
}

#[doc(hidden)]
#[must_use]
pub fn compile_glob_set_for_fuzz(patterns: &[String]) -> GlobSet {
    compile_glob_set(patterns)
}

pub fn normalize_http_scheme(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else if let Some(rest) = url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else {
        url.to_string()
    }
}

/// Canonicalize an HTTP(S)/WS(S) URL for policy matching.
///
/// The URL is parsed with the `url` crate so userinfo, query and fragment can
/// never take part in host matching. Result:
/// `scheme://host[:port]/path[?query]` with the scheme and host lowercased,
/// `ws`/`wss` mapped to `http`/`https`, the default port and a trailing host
/// dot removed, and the path left as-is. URLs with userinfo, backslashes or
/// control characters are rejected. IPv4-mapped (`::ffff:a.b.c.d`) and
/// IPv4-compatible (`::a.b.c.d`) IPv6 hosts are rewritten to plain IPv4.
///
/// Host deny rules are NOT an SSRF boundary: matching is purely textual, so a
/// DNS name that resolves to a loopback or private address (or a rebinding
/// record) is not caught by a deny rule on that IP. Enforce network-level
/// restrictions on the resolved address if that is the goal.
pub fn canonicalize_http_url(url: &str) -> Result<String, String> {
    http_candidates(url).map(|(full, _)| full)
}

/// Returns `(with_query, path_only)` canonical forms (fragment always dropped).
fn http_candidates(raw: &str) -> Result<(String, String), String> {
    let invalid = || format!("http: invalid URL '{raw}'");
    if raw.contains('\\') || raw.chars().any(char::is_control) || raw.trim() != raw {
        return Err(invalid());
    }
    let parsed = url::Url::parse(raw).map_err(|_| invalid())?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(format!(
            "http: URLs with embedded credentials are not allowed - '{raw}'"
        ));
    }
    let scheme = match parsed.scheme() {
        "wss" => "https",
        "ws" => "http",
        other => other,
    };
    let Some(host) = parsed.host_str() else {
        let mut opaque = parsed.clone();
        opaque.set_fragment(None);
        let text = opaque.to_string();
        return Ok((text.clone(), text));
    };
    let host = match parsed.host() {
        Some(url::Host::Ipv6(addr)) => {
            ipv6_as_ipv4(addr).map_or_else(|| host.to_ascii_lowercase(), |v4| v4.to_string())
        },
        _ => host.trim_end_matches('.').to_ascii_lowercase(),
    };
    let port = parsed.port().map_or_else(String::new, |p| format!(":{p}"));
    let base = format!(
        "{scheme}://{host}{port}{}",
        collapse_slashes(&normalize_percent_encoding(parsed.path()))
    );
    let full = match parsed.query() {
        Some(query) => format!("{base}?{}", normalize_percent_encoding(query)),
        None => base.clone(),
    };
    Ok((full, base))
}

/// IPv4 address an IPv6 host stands for: IPv4-mapped (`::ffff:a.b.c.d`) or
/// IPv4-compatible (`::a.b.c.d`). `::1` is the IPv6 loopback and never mapped.
fn ipv6_as_ipv4(addr: std::net::Ipv6Addr) -> Option<std::net::Ipv4Addr> {
    if addr == std::net::Ipv6Addr::LOCALHOST {
        return None;
    }
    addr.to_ipv4()
}

/// Canonical percent-encoding for the path/query part of a URL or pattern.
///
/// - `%XX` escapes of unreserved characters (`A-Za-z0-9-._~`) are decoded, so
///   `/%61dmin` and `/admin` compare equal.
/// - Every other escape keeps its encoding (notably `%2F` stays an escaped
///   slash and never becomes a path separator), with uppercase hex digits.
/// - Raw bytes that URL parsing would encode (non-ASCII, space, controls and
///   `"<>` `` ` `` `^`) are encoded, so a pattern written with a literal `é` or
///   space matches the `%C3%A9` / `%20` form of the candidate.
///
/// Glob metacharacters and other literal ASCII are left untouched.
fn normalize_percent_encoding(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            let hi = bytes.get(i + 1).and_then(|c| (*c as char).to_digit(16));
            let lo = bytes.get(i + 2).and_then(|c| (*c as char).to_digit(16));
            if let (Some(hi), Some(lo)) = (hi, lo) {
                let decoded = u8::try_from(hi * 16 + lo).unwrap_or(b'%');
                if decoded.is_ascii_alphanumeric() || matches!(decoded, b'-' | b'.' | b'_' | b'~') {
                    out.push(char::from(decoded));
                } else {
                    out.push('%');
                    out.push(char::from(HEX[usize::from(decoded >> 4)]));
                    out.push(char::from(HEX[usize::from(decoded & 0xF)]));
                }
                i += 3;
                continue;
            }
            out.push('%');
        } else if b >= 0x80
            || b <= 0x20
            || b == 0x7F
            || matches!(b, b'"' | b'<' | b'>' | b'`' | b'^')
        {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0xF)]));
        } else {
            out.push(char::from(b));
        }
        i += 1;
    }
    out
}

/// Normalize an HTTP allow/deny pattern to match [`canonicalize_http_url`]:
/// lowercase scheme and authority, drop an explicit default port, give an
/// empty path the `/` that URL parsing always produces, and canonicalize the
/// percent-encoding of the path with [`normalize_percent_encoding`].
fn normalize_http_pattern(pat: &str) -> String {
    let Some((scheme, rest)) = pat.split_once("://") else {
        return pat.to_string();
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    // Candidates map ws/wss to http/https, so patterns must too.
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "wss" => "https".to_string(),
        "ws" => "http".to_string(),
        other => other.to_string(),
    };
    let mut authority = normalize_pattern_authority(authority);
    let default_port = match scheme.as_str() {
        "https" => ":443",
        "http" => ":80",
        _ => "",
    };
    if !default_port.is_empty() && authority.ends_with(default_port) {
        authority.truncate(authority.len() - default_port.len());
    }
    format!(
        "{scheme}://{authority}{}",
        collapse_slashes(&normalize_percent_encoding(path))
    )
}

/// Lowercase a pattern authority and convert non-ASCII host labels to
/// punycode, the form `url::Url` gives candidates. Labels holding glob
/// metacharacters are only lowercased.
fn normalize_pattern_authority(authority: &str) -> String {
    if let Some(rest) = authority.strip_prefix('[') {
        let mapped = rest.split_once(']').and_then(|(inner, tail)| {
            let v4 = ipv6_as_ipv4(inner.parse::<std::net::Ipv6Addr>().ok()?)?;
            Some(format!("{v4}{}", tail.to_lowercase()))
        });
        if let Some(mapped) = mapped {
            return mapped;
        }
        return authority.to_lowercase();
    }
    if authority.is_ascii() {
        return authority.to_lowercase();
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        _ => (authority, None),
    };
    let host = host
        .split('.')
        .map(|label| {
            let has_glob = label.contains(['*', '?', '[', ']', '{', '}']);
            if label.is_ascii() || has_glob {
                return label.to_lowercase();
            }
            match url::Host::parse(label) {
                Ok(url::Host::Domain(ascii)) => ascii,
                _ => label.to_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join(".");
    match port {
        Some(port) => format!("{host}:{}", port.to_lowercase()),
        None => host,
    }
}

/// Lowercase fragments that make a path mean different things to different
/// servers: encoded slash/backslash, double-encoded dot/slash/backslash, and
/// `;` path parameters (stripped per segment by Tomcat/Spring, so `..;` is a
/// dot segment there).
///
/// `%00` (NUL truncation) is also a marker, and so is a `%` that is not
/// followed by two hex digits ([`MALFORMED_PERCENT`]): IIS/ASP.NET decode
/// `%uXXXX` and other lenient forms that a strict parser leaves alone.
const AMBIGUOUS_PATH_MARKERS: [&str; 7] = ["%2f", "%5c", "%252e", "%252f", "%255c", ";", "%00"];

/// Synthetic marker for a `%` not followed by two hex digits.
const MALFORMED_PERCENT: &str = "%<malformed>";

/// True when `text` (any case) holds a `%` not followed by two hex digits.
fn has_malformed_percent(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(i, b)| {
        *b == b'%'
            && !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    })
}

/// Markers present in a lowercase candidate path.
fn ambiguous_markers(lower_path: &str) -> Vec<&'static str> {
    let mut markers: Vec<&'static str> = AMBIGUOUS_PATH_MARKERS
        .iter()
        .copied()
        .filter(|needle| lower_path.contains(needle))
        .collect();
    if has_malformed_percent(lower_path) {
        markers.push(MALFORMED_PERCENT);
    }
    markers
}

/// True when the lowercase allow rule spells out `marker`.
fn rule_spells_marker(lower_rule: &str, marker: &str) -> bool {
    if marker == MALFORMED_PERCENT {
        has_malformed_percent(lower_rule)
    } else {
        lower_rule.contains(marker)
    }
}

/// Collapse runs of `/` (servers such as nginx and Express merge them).
fn collapse_slashes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut previous_slash = false;
    for c in path.chars() {
        if c == '/' && previous_slash {
            continue;
        }
        previous_slash = c == '/';
        out.push(c);
    }
    out
}

/// True when the candidate path could be read as a different path by a server
/// or proxy: see [`AMBIGUOUS_PATH_MARKERS`], or a leftover dot segment.
fn path_has_encoded_separator_or_dot_segment(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if !ambiguous_markers(&lower).is_empty() {
        return true;
    }
    path.split('/').any(|segment| matches!(segment, "." | ".."))
}

/// Path component of a canonical `scheme://host[:port]/path` candidate.
fn candidate_path(canonical: &str) -> &str {
    canonical
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|i| &rest[i..]))
        .unwrap_or("")
}

pub fn canonicalize_process_binary(binary: &str) -> String {
    let normalized = std::path::Path::new(binary)
        .to_string_lossy()
        .replace('\\', "/");
    let resolved = dunce::canonicalize(binary).ok().map_or_else(
        || normalized.clone(),
        |p| p.to_string_lossy().replace('\\', "/"),
    );
    if cfg!(target_os = "windows") {
        fold_path_case(&resolved)
    } else {
        resolved
    }
}

pub fn canonicalize_fs(path: &str) -> String {
    let cleaned = if cfg!(target_os = "windows") {
        lex_clean_str(&strip_win32_segment_suffixes(&path.replace('\\', "/")))
    } else {
        lex_clean_str(path)
    };
    let absolute = {
        let p = std::path::Path::new(&cleaned);
        let already_rooted = p.is_absolute()
            || p.has_root()
            || cleaned.starts_with('/')
            || cleaned.starts_with(std::path::MAIN_SEPARATOR);
        if already_rooted {
            cleaned
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(p).to_string_lossy().into_owned())
                .map(|joined| lex_clean_str(&joined))
                .unwrap_or(cleaned)
        }
    };
    let normalized = absolute.replace('\\', "/");
    if cfg!(target_os = "windows") {
        fold_path_case(&normalized)
    } else {
        normalized
    }
}

/// Win32 path normalization, applied before matching on Windows: every
/// segment loses its alternate data stream suffix (`grants.json::$DATA`,
/// `.ssh:$I30:$INDEX_ALLOCATION`) and its trailing dots and spaces
/// (`security.\grants.json` and `.ssh ` open `security\grants.json` and
/// `.ssh`). A segment left empty is dropped; `.`, `..` and drive prefixes
/// (`C:`, `\\?\C:`) are kept. Expects forward slashes.
#[must_use]
pub fn strip_win32_segment_suffixes(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let device_prefix = segments.len() > 3
        && segments.first().is_some_and(|s| s.is_empty())
        && segments.get(1).is_some_and(|s| s.is_empty())
        && segments.get(2).is_some_and(|s| *s == "?" || *s == ".");
    let mut out: Vec<String> = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() || *segment == "." || *segment == ".." || (device_prefix && index < 3)
        {
            out.push((*segment).to_string());
            continue;
        }
        let may_hold_drive = index == 0 || (device_prefix && index == 3);
        let bytes = segment.as_bytes();
        let (drive, rest) = if may_hold_drive
            && bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && bytes.get(1) == Some(&b':')
        {
            segment.split_at(2)
        } else {
            ("", *segment)
        };
        let stream_free = rest.split(':').next().unwrap_or_default();
        let trimmed = stream_free.trim_end_matches(['.', ' ']);
        if trimmed.is_empty() && drive.is_empty() {
            continue;
        }
        out.push(format!("{drive}{trimmed}"));
    }
    out.join("/")
}

pub fn lex_clean_str(path: &str) -> String {
    use std::path::{Component, Path, PathBuf};
    let mut out = PathBuf::new();
    for c in Path::new(path).components() {
        match c {
            Component::ParentDir => {
                out.pop();
            },
            Component::CurDir => {},
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().into_owned()
}

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

pub fn matches_pattern(pattern: &str, input: &str) -> bool {
    let pat = if cfg!(target_os = "windows") {
        fold_path_case(&pattern.replace('\\', "/"))
    } else {
        pattern.to_string()
    };
    globset::GlobBuilder::new(&pat)
        .literal_separator(true)
        .build()
        .is_ok_and(|g| g.compile_matcher().is_match(input))
}

/// Characters that let a command line escape the first-token allowlist.
/// `%` (cmd.exe variable expansion), `^` (cmd.exe escape) and `!` (delayed
/// expansion / history) are included for Windows and interactive shells.
const SHELL_METACHARS: &[char] = &[
    '&', '|', ';', '<', '>', '(', ')', '{', '}', '\'', '"', '`', '\\', '$', '\n', '\r', '%', '^',
    '!',
];

/// Exact variable names (compared ASCII case-insensitively) that let a caller
/// change what an allowlisted binary loads or runs.
const ENV_DENYLIST: &[&str] = &[
    // Search path / shell resolution.
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "PSMODULEPATH",
    "CDPATH",
    "IFS",
    // Shell startup hooks.
    "BASH_ENV",
    "ENV",
    "PROMPT_COMMAND",
    "PS4",
    "SHELLOPTS",
    "BASHOPTS",
    // Loader / locale hooks (LD_* and DYLD_* are prefixes).
    "GCONV_PATH",
    "LOCPATH",
    "NLSPATH",
    "_NT_SYMBOL_PATH",
    "__COMPAT_LAYER",
    // Node / Bun / Electron.
    "NODE_OPTIONS",
    "NODE_PATH",
    "NODE_EXTRA_CA_CERTS",
    "NODE_REPL_EXTERNAL_MODULE",
    "ELECTRON_RUN_AS_NODE",
    "BUN_INSTALL",
    "BUN_OPTIONS",
    "BUN_RUNTIME_OPTIONS",
    "BUN_INSPECT",
    // Git (GIT_CONFIG* and GIT_SSH* are prefixes).
    "GIT_EXEC_PATH",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_EXTERNAL_DIFF",
    "GIT_PAGER",
    "GIT_EDITOR",
    "GIT_SEQUENCE_EDITOR",
    "GIT_PROXY_COMMAND",
    "GIT_TEMPLATE_DIR",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_NAMESPACE",
    // Interpreters.
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "PYTHONINSPECT",
    "PYTHONUSERBASE",
    "PYTHONBREAKPOINT",
    "PYTHONEXECUTABLE",
    "PYTHONPYCACHEPREFIX",
    "PERL5LIB",
    "PERL5OPT",
    "PERL5DB",
    "PERLLIB",
    "RUBYOPT",
    "RUBYLIB",
    "JAVA_TOOL_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "CLASSPATH",
    // Editors / pagers spawned by tools.
    "EDITOR",
    "VISUAL",
    "PAGER",
    "MANPAGER",
    "LESSOPEN",
    "LESSCLOSE",
    // .NET / PowerShell assembly injection.
    "DOTNET_STARTUP_HOOKS",
    "DOTNET_ADDITIONAL_DEPS",
    "DOTNET_SHARED_STORE",
    // Known Python exec gadget (webbrowser + warnings) and engine/module DLLs.
    "PYTHONWARNINGS",
    "BROWSER",
    "OPENSSL_CONF",
    "OPENSSL_ENGINES",
    "OPENSSL_MODULES",
    // Config redirection (git, ssh, zsh read their config from here).
    "ZDOTDIR",
    "HOME",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_CONFIG_DIRS",
    "XDG_DATA_HOME",
    "XDG_DATA_DIRS",
];

/// Variable-name prefixes (ASCII case-insensitive) that are denied wholesale.
const ENV_DENY_PREFIXES: &[&str] = &[
    "LD_",
    "DYLD_",
    "GIT_CONFIG",
    "GIT_SSH",
    "_JAVA_",
    "NPM_CONFIG_",
    // Exported bash functions.
    "BASH_FUNC_",
    // .NET runtime profiler injection.
    "COR_ENABLE_PROFILING",
    "COR_PROFILER",
    "CORECLR_",
];

#[must_use]
pub fn env_key_denied(key: &str) -> bool {
    if ENV_DENYLIST.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        return true;
    }
    ENV_DENY_PREFIXES.iter().any(|prefix| {
        key.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    })
}

#[cfg(test)]
mod tests;
