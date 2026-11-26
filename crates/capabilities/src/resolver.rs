use std::collections::BTreeMap;

use globset::{Glob, GlobMatcher};

use crate::{
    manifest::{command_lookup_keys, manifest_command_key, resolve_manifest_permissions},
    Capability, Manifest, Permission, PermissionEntry, Target,
};

pub const APP_ACL_KEY: &str = "__app__";

#[derive(Debug)]
pub struct Resolved {
    allowed_commands: BTreeMap<String, Vec<ResolvedCommand>>,
    denied_commands: BTreeMap<String, Vec<ResolvedCommand>>,
}

impl Resolved {
    pub fn resolve(
        manifests: &BTreeMap<String, Manifest>,
        capabilities: BTreeMap<String, Capability>,
        target: Target,
    ) -> Result<Self, String> {
        let mut out = Self {
            allowed_commands: BTreeMap::new(),
            denied_commands: BTreeMap::new(),
        };

        for cap in capabilities.values() {
            if !platforms_match(cap.platforms.as_deref(), target) {
                continue;
            }
            let context = ResolvedCommand::from_capability(cap)?;
            for entry in &cap.permissions {
                out.add_entry(manifests, entry, &context, target)?;
            }
        }

        Ok(out)
    }

    pub fn is_allowed(
        &self,
        command: &str,
        window: Option<&str>,
        webview: Option<&str>,
        origin: Option<&str>,
    ) -> bool {
        let lookup_keys = command_lookup_keys(command);
        let matches = |commands: &BTreeMap<String, Vec<ResolvedCommand>>| {
            lookup_keys
                .iter()
                .filter_map(|key| commands.get(key.as_str()))
                .flatten()
                .any(|cmd| context_matches(cmd, window, webview, origin))
        };

        // Deny wins over allow, whichever lookup key matched.
        !matches(&self.denied_commands) && matches(&self.allowed_commands)
    }

    /// Add one capability permission entry: a `manifest:permission`
    /// reference (expanded through its manifest) or an inline permission.
    fn add_entry(
        &mut self,
        manifests: &BTreeMap<String, Manifest>,
        entry: &PermissionEntry,
        context: &ResolvedCommand,
        target: Target,
    ) -> Result<(), String> {
        match entry {
            PermissionEntry::PermissionRef(reference) => {
                let (manifest_name, permission_id) = split_permission_ref(reference);
                let manifest = manifests.get(manifest_name).ok_or_else(|| {
                    format!("unknown manifest '{manifest_name}' for permission '{reference}'")
                })?;
                for permission in
                    resolve_manifest_permissions(manifest_name, manifest, permission_id)?
                {
                    if platforms_match(permission.platforms.as_deref(), target) {
                        self.add_permission(permission, context, |cmd| {
                            manifest_command_key(manifest_name, cmd)
                        });
                    }
                }
            },
            PermissionEntry::Inline(permission) => {
                if platforms_match(permission.platforms.as_deref(), target) {
                    self.add_permission(permission, context, str::to_string);
                }
            },
        }
        Ok(())
    }

    /// Record `context` under the key of every allowed and denied command of
    /// `permission`; `key` maps a command name to its lookup key.
    fn add_permission(
        &mut self,
        permission: &Permission,
        context: &ResolvedCommand,
        key: impl Fn(&str) -> String,
    ) {
        for cmd in &permission.commands.allow {
            self.allowed_commands
                .entry(key(cmd))
                .or_default()
                .push(context.clone());
        }
        for cmd in &permission.commands.deny {
            self.denied_commands
                .entry(key(cmd))
                .or_default()
                .push(context.clone());
        }
    }
}

#[derive(Debug, Clone)]
struct ResolvedCommand {
    windows: Vec<CompiledPattern>,
    webviews: Vec<CompiledPattern>,
    local: bool,
    remote: Vec<RemotePattern>,
}

impl ResolvedCommand {
    fn from_capability(cap: &Capability) -> Result<Self, String> {
        Ok(Self {
            windows: compile_patterns(&cap.windows)?,
            webviews: compile_patterns(&cap.webviews)?,
            local: cap.local,
            remote: cap
                .remote
                .as_ref()
                .map(|r| compile_remote_patterns(&r.urls))
                .transpose()?
                .unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone)]
struct CompiledPattern {
    matcher: GlobMatcher,
}

impl CompiledPattern {
    fn matches(&self, value: &str) -> bool {
        self.matcher.is_match(value)
    }
}

#[derive(Debug, Clone)]
struct RemotePattern {
    matcher: GlobMatcher,
}

impl RemotePattern {
    fn matches(&self, url: &url::Url) -> bool {
        self.matcher.is_match(url.as_str())
    }
}

fn compile_patterns(patterns: &[String]) -> Result<Vec<CompiledPattern>, String> {
    patterns
        .iter()
        .map(|pattern| {
            Glob::new(pattern)
                .map(|glob| CompiledPattern {
                    matcher: glob.compile_matcher(),
                })
                .map_err(|e| format!("invalid glob pattern '{pattern}': {e}"))
        })
        .collect()
}

fn compile_remote_patterns(patterns: &[String]) -> Result<Vec<RemotePattern>, String> {
    patterns
        .iter()
        .map(|pattern| {
            Glob::new(pattern)
                .map(|glob| RemotePattern {
                    matcher: glob.compile_matcher(),
                })
                .map_err(|e| format!("invalid remote URL pattern '{pattern}': {e}"))
        })
        .collect()
}

pub fn platforms_match(platforms: Option<&[Target]>, target: Target) -> bool {
    platforms.is_none_or(|allowed| allowed.contains(&target))
}

pub fn split_permission_ref(reference: &str) -> (&str, &str) {
    reference
        .split_once(':')
        .map_or((APP_ACL_KEY, reference), |(plugin, permission)| {
            (plugin, permission)
        })
}

fn context_matches(
    cmd: &ResolvedCommand,
    window: Option<&str>,
    webview: Option<&str>,
    origin: Option<&str>,
) -> bool {
    let origin_ok = match origin {
        None => cmd.local,
        Some(origin) => match origin.parse::<url::Url>().ok() {
            Some(parsed) => cmd.remote.iter().any(|pattern| pattern.matches(&parsed)),
            None => false,
        },
    };
    if !origin_ok {
        return false;
    }

    let window_ok = cmd.windows.is_empty()
        || window.is_some_and(|value| cmd.windows.iter().any(|pattern| pattern.matches(value)));
    let webview_ok = cmd.webviews.is_empty()
        || webview.is_some_and(|value| cmd.webviews.iter().any(|pattern| pattern.matches(value)));

    // Every declared scope narrows the grant. Treating `windows` and
    // `webviews` as alternatives lets any child inherit a parent-window
    // grant even when the capability explicitly names another webview.
    window_ok && webview_ok
}
