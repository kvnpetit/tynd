//! Runtime scope grants: paths and URL patterns the trusted backend (or a
//! native file dialog the user answered) adds to the filesystem and HTTP
//! scopes after startup.
//!
//! Grants only ever *add* reach on top of the declarative policy, and only
//! where they name it: configured deny rules always win, a read-only path
//! grant never authorizes a write, and an HTTP grant never admits a URL whose
//! path carries an encoded separator or dot segment. The host resolves every
//! path through the kernel before it reaches this module; matching here is a
//! pure, component-wise comparison of canonical spellings.

use serde::{Deserialize, Serialize};

use crate::{
    candidate_path, canonicalize_fs, http_candidates, path_has_encoded_separator_or_dot_segment,
    Scope, SecurityPolicy,
};

/// Most path grants held at once.
pub const MAX_PATH_GRANTS: usize = 1024;
/// Most URL grants held at once.
pub const MAX_URL_GRANTS: usize = 256;
/// Longest accepted path or URL pattern, in bytes.
const MAX_GRANT_LEN: usize = 4096;

/// What a filesystem operation does with the path it is authorized for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FsAccess {
    /// Reads metadata or content only.
    Read,
    /// Creates, modifies, moves or deletes, or hands the path to another
    /// program. The default for any caller that does not say otherwise.
    #[default]
    Write,
}

/// One granted path. `path` is the kernel-resolved spelling the host computed
/// when the grant was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PathGrant {
    pub path: String,
    /// The grant names a directory: its entries are covered too (direct
    /// children only unless `recursive`).
    #[serde(default)]
    pub directory: bool,
    /// Cover the whole subtree of a directory grant.
    #[serde(default)]
    pub recursive: bool,
    /// Also authorize writes (create, modify, delete). Read-only otherwise.
    #[serde(default)]
    pub write: bool,
}

impl PathGrant {
    fn key(&self) -> String {
        canonicalize_fs(&self.path)
    }

    /// Whether this grant covers the canonical spelling `candidate` for
    /// `access`.
    fn covers(&self, key: &str, candidate: &str, access: FsAccess) -> bool {
        if access == FsAccess::Write && !self.write {
            return false;
        }
        if candidate == key {
            return true;
        }
        if !self.directory {
            return false;
        }
        let rest = if key.ends_with('/') {
            candidate.strip_prefix(key)
        } else {
            candidate
                .strip_prefix(key)
                .and_then(|rest| rest.strip_prefix('/'))
        };
        rest.is_some_and(|rest| !rest.is_empty() && (self.recursive || !rest.contains('/')))
    }
}

/// Serializable view of every grant (`list()` and the persisted file).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GrantsSnapshot {
    #[serde(default)]
    pub paths: Vec<PathGrant>,
    #[serde(default)]
    pub urls: Vec<String>,
}

/// The live grant set.
#[derive(Debug, Clone, Default)]
pub struct Grants {
    paths: Vec<(String, PathGrant)>,
    urls: Vec<String>,
    url_scope: Scope,
}

impl Grants {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.urls.is_empty()
    }

    /// Add (or replace: the latest grant for a path wins) one path grant.
    /// `grant.path` must be absolute and already kernel-resolved.
    pub fn add_path(&mut self, grant: PathGrant) -> Result<(), String> {
        validate_grant_path(&grant.path)?;
        if grant.recursive && !grant.directory {
            return Err(format!(
                "security.allowPath: 'recursive' needs a directory, '{}' is a file",
                grant.path
            ));
        }
        let key = grant.key();
        if let Some(slot) = self.paths.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = grant;
            return Ok(());
        }
        if self.paths.len() >= MAX_PATH_GRANTS {
            return Err(format!(
                "security.allowPath: at most {MAX_PATH_GRANTS} path grants - revoke one first"
            ));
        }
        self.paths.push((key, grant));
        Ok(())
    }

    /// Remove the grant made for `path` (same resolved spelling). Returns
    /// whether one existed.
    pub fn revoke_path(&mut self, path: &str) -> bool {
        let key = canonicalize_fs(path);
        let before = self.paths.len();
        self.paths.retain(|(k, _)| *k != key);
        self.paths.len() != before
    }

    /// Add one `http(s)://` URL glob (same syntax as `security.http.allow`).
    pub fn add_url(&mut self, pattern: &str) -> Result<(), String> {
        validate_url_pattern(pattern)?;
        if self.urls.iter().any(|existing| existing == pattern) {
            return Ok(());
        }
        if self.urls.len() >= MAX_URL_GRANTS {
            return Err(format!(
                "security.allowUrl: at most {MAX_URL_GRANTS} URL grants - revoke one first"
            ));
        }
        let mut urls = self.urls.clone();
        urls.push(pattern.to_string());
        self.url_scope = Scope::try_new_http(urls.clone(), Vec::new())?;
        self.urls = urls;
        Ok(())
    }

    /// Remove a URL pattern added by [`Self::add_url`]. Returns whether it
    /// existed.
    pub fn revoke_url(&mut self, pattern: &str) -> bool {
        let before = self.urls.len();
        self.urls.retain(|existing| existing != pattern);
        if self.urls.len() == before {
            return false;
        }
        self.url_scope = Scope::new_http(self.urls.clone(), Vec::new());
        true
    }

    /// Whether a grant covers the kernel-resolved `path` for `access`.
    pub fn allows_path(&self, path: &str, access: FsAccess) -> bool {
        if self.paths.is_empty() {
            return false;
        }
        let candidate = canonicalize_fs(path);
        self.paths
            .iter()
            .any(|(key, grant)| grant.covers(key, &candidate, access))
    }

    /// The most specific grant (longest path) covering `path` for `access`.
    pub fn covering(&self, path: &str, access: FsAccess) -> Option<&PathGrant> {
        let candidate = canonicalize_fs(path);
        self.paths
            .iter()
            .filter(|(key, grant)| grant.covers(key, &candidate, access))
            .max_by_key(|(key, _)| key.len())
            .map(|(_, grant)| grant)
    }

    /// Whether a URL grant covers `url`. Paths with an encoded separator or a
    /// dot segment are never granted.
    pub fn allows_url(&self, url: &str) -> bool {
        if self.urls.is_empty() {
            return false;
        }
        let Ok((full, path_only)) = http_candidates(url) else {
            return false;
        };
        if path_has_encoded_separator_or_dot_segment(candidate_path(&path_only)) {
            return false;
        }
        self.url_scope
            .check_any(&[&full, &path_only], true, "http")
            .is_ok()
    }

    pub fn snapshot(&self) -> GrantsSnapshot {
        GrantsSnapshot {
            paths: self.paths.iter().map(|(_, grant)| grant.clone()).collect(),
            urls: self.urls.clone(),
        }
    }
}

fn validate_grant_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_GRANT_LEN || path.chars().any(char::is_control) {
        return Err(format!("security.allowPath: invalid path '{path}'"));
    }
    if !std::path::Path::new(path).is_absolute() {
        return Err(format!(
            "security.allowPath: path must be absolute - '{path}'"
        ));
    }
    Ok(())
}

/// A URL grant must name an `http(s)://` origin: a bare `**` would silently
/// open every host.
pub fn validate_url_pattern(pattern: &str) -> Result<(), String> {
    if pattern.len() > MAX_GRANT_LEN || pattern.chars().any(char::is_control) {
        return Err(format!("security.allowUrl: invalid pattern '{pattern}'"));
    }
    let rest = pattern
        .strip_prefix("https://")
        .or_else(|| pattern.strip_prefix("http://"))
        .ok_or_else(|| {
            format!("security.allowUrl: pattern must start with https:// or http:// - '{pattern}'")
        })?;
    let host = rest.split(['/', '?']).next().unwrap_or("");
    if host.is_empty() || host.trim_start_matches("*.").contains('*') || host.contains('@') {
        return Err(format!(
            "security.allowUrl: pattern must name a host (only a leading '*.' wildcard) - \
             '{pattern}'"
        ));
    }
    Scope::try_new_http(vec![pattern.to_string()], Vec::new())
        .map(drop)
        .map_err(|error| format!("security.allowUrl: {error}"))
}

impl SecurityPolicy {
    /// Filesystem check of a kernel-resolved path with runtime grants.
    ///
    /// `in_roots` says whether the path lies inside the host's capability
    /// roots. Inside them the declarative policy decides first and a grant can
    /// only admit what an allow-list left out; outside them only an explicit
    /// `fs.allow` rule or a grant can admit the path (an empty allow list
    /// admits nothing there, even while no policy is configured). Deny rules
    /// win in both cases.
    pub fn check_fs_with_grants(
        &self,
        path: &str,
        grants: &Grants,
        access: FsAccess,
        in_roots: bool,
    ) -> Result<(), String> {
        let canonical = canonicalize_fs(path);
        if self.configured && self.fs.deny_set.is_match(&canonical) {
            return Err(format!("fs: denied by security policy - '{canonical}'"));
        }
        if in_roots {
            let base = self.check_fs_resolved(path);
            if base.is_ok() || grants.allows_path(path, access) {
                return Ok(());
            }
            return base;
        }
        if self.fs_allow_rule_matches(path) || grants.allows_path(path, access) {
            return Ok(());
        }
        Err(match access {
            FsAccess::Write if grants.allows_path(path, FsAccess::Read) => {
                format!("fs: '{canonical}' was granted read-only")
            },
            _ => format!("fs: no grant covers '{canonical}'"),
        })
    }

    /// [`Self::check_http`] with runtime URL grants added to a configured
    /// allow-list. Grants never override a deny rule.
    pub fn check_http_with_grants(&self, url: &str, grants: &Grants) -> Result<(), String> {
        if !self.configured {
            return Ok(());
        }
        self.check_http_granted(url, grants)
    }

    /// [`Self::check_http_configured`] with runtime URL grants.
    pub fn check_http_configured_with_grants(
        &self,
        url: &str,
        grants: &Grants,
    ) -> Result<(), String> {
        if !self.configured {
            return Err("http: security policy is not configured".into());
        }
        self.check_http_granted(url, grants)
    }

    fn check_http_granted(&self, url: &str, grants: &Grants) -> Result<(), String> {
        let base = self.check_http_inner(url);
        if base.is_ok() || !grants.allows_url(url) {
            return base;
        }
        let (full, path_only) = http_candidates(url)?;
        if self.http.deny_set.is_match(&full) || self.http.deny_set.is_match(&path_only) {
            return base;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
