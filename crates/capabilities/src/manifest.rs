use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU64,
};

use serde_json::Value;

use crate::{Commands, Manifest, MethodName, Permission, PluginName, APP_ACL_KEY};

const PLUGIN_KEY_PREFIX: &str = "plugin:";

pub(crate) fn resolve_manifest_permissions<'a>(
    manifest_name: &str,
    manifest: &'a Manifest,
    requested: &str,
) -> Result<Vec<&'a Permission>, String> {
    let root = if requested == "default" {
        manifest.default_permission.as_deref().ok_or_else(|| {
            format!("manifest '{manifest_name}' does not declare a default permission")
        })?
    } else {
        requested
    };

    let mut resolved = Vec::new();
    let mut resolved_ids = BTreeSet::new();
    let mut stack = Vec::new();
    expand_manifest_entry(
        manifest_name,
        manifest,
        root,
        &mut stack,
        &mut resolved_ids,
        &mut resolved,
    )?;
    Ok(resolved)
}

fn expand_manifest_entry<'a>(
    manifest_name: &str,
    manifest: &'a Manifest,
    entry: &str,
    stack: &mut Vec<String>,
    resolved_ids: &mut BTreeSet<String>,
    resolved: &mut Vec<&'a Permission>,
) -> Result<(), String> {
    let entry = normalize_manifest_entry(manifest_name, entry)?;

    if let Some(permission) = manifest.permissions.get(entry) {
        if manifest.permission_sets.contains_key(entry) {
            return Err(format!(
                "manifest '{manifest_name}' declares both a permission and permission set named \
                 '{entry}'"
            ));
        }
        if resolved_ids.insert(entry.to_string()) {
            resolved.push(permission);
        }
        return Ok(());
    }

    let members = manifest.permission_sets.get(entry).ok_or_else(|| {
        format!("unknown permission or permission set '{entry}' in manifest '{manifest_name}'")
    })?;
    if let Some(cycle_start) = stack.iter().position(|candidate| candidate == entry) {
        let mut cycle = stack[cycle_start..].to_vec();
        cycle.push(entry.to_string());
        return Err(format!(
            "permission set cycle in manifest '{manifest_name}': {}",
            cycle.join(" -> ")
        ));
    }

    stack.push(entry.to_string());
    for member in members {
        expand_manifest_entry(
            manifest_name,
            manifest,
            member,
            stack,
            resolved_ids,
            resolved,
        )?;
    }
    stack.pop();
    Ok(())
}

fn normalize_manifest_entry<'a>(manifest_name: &str, entry: &'a str) -> Result<&'a str, String> {
    let Some((prefix, local)) = entry.split_once(':') else {
        return Ok(entry);
    };
    if prefix == manifest_name {
        Ok(local)
    } else {
        Err(format!(
            "permission set in manifest '{manifest_name}' cannot reference foreign manifest \
             '{prefix}'"
        ))
    }
}

pub fn manifest_command_key(manifest_name: &str, command: &str) -> String {
    if manifest_name == APP_ACL_KEY {
        command.to_string()
    } else {
        format!("{PLUGIN_KEY_PREFIX}{manifest_name}|{command}")
    }
}

pub fn plugin_command_key(plugin: &str, method: &str) -> Result<String, String> {
    let plugin = validate_plugin_name(plugin)?;
    let method = MethodName::try_new(method)
        .map_err(|error| format!("invalid plugin command name '{method}': {error}"))?;
    Ok(manifest_command_key(plugin.as_ref(), method.as_ref()))
}

pub fn command_lookup_keys(command: &str) -> Vec<String> {
    let (wildcard, method) = if let Some((plugin, method)) = parse_plugin_command_key(command) {
        (manifest_command_key(plugin, "*"), method)
    } else if let Some((api, method)) = command.split_once(':') {
        (format!("{api}:*"), method)
    } else {
        return vec![command.to_string()];
    };
    if method == "*" {
        vec![wildcard]
    } else {
        vec![command.to_string(), wildcard]
    }
}

pub fn manifest_from_plugin_commands(
    plugin_name: &str,
    command_names: &[&str],
) -> Result<Option<Manifest>, String> {
    validate_plugin_name(plugin_name)?;
    if command_names.is_empty() {
        return Ok(None);
    }

    let mut permissions = BTreeMap::new();
    let mut allow_all = Vec::with_capacity(command_names.len());
    let mut deny_all = Vec::with_capacity(command_names.len());
    let mut command_ids = BTreeSet::new();

    for command in command_names {
        MethodName::try_new(*command)
            .map_err(|error| format!("invalid command name '{command}': {error}"))?;
        let suffix = command_permission_suffix(command);
        if !command_ids.insert(suffix.clone()) {
            return Err(format!(
                "commands produce duplicate permission identifier '{suffix}'"
            ));
        }

        let allow_id = format!("allow-{suffix}");
        let deny_id = format!("deny-{suffix}");
        allow_all.push(allow_id.clone());
        deny_all.push(deny_id.clone());
        permissions.insert(
            allow_id.clone(),
            single_command_permission(allow_id, command, true),
        );
        permissions.insert(
            deny_id.clone(),
            single_command_permission(deny_id, command, false),
        );
    }

    Ok(Some(Manifest {
        default_permission: Some("allow-all".into()),
        permissions,
        permission_sets: BTreeMap::from([
            ("allow-all".into(), allow_all),
            ("deny-all".into(), deny_all),
        ]),
        global_scope_schema: None,
    }))
}

/// Generated `allow-*` (`allow: true`) or `deny-*` permission covering
/// exactly `command`.
fn single_command_permission(identifier: String, command: &str, allow: bool) -> Permission {
    let listed = vec![command.to_string()];
    let (verb, commands) = if allow {
        (
            "Allows",
            Commands {
                allow: listed,
                deny: vec![],
            },
        )
    } else {
        (
            "Denies",
            Commands {
                allow: vec![],
                deny: listed,
            },
        )
    };
    Permission {
        version: NonZeroU64::new(1),
        identifier,
        description: Some(format!("{verb} the '{command}' command.")),
        commands,
        scope: Value::Null,
        platforms: None,
    }
}

pub fn validate_plugin_name(plugin_name: &str) -> Result<PluginName, String> {
    if plugin_name == APP_ACL_KEY {
        return Err(format!(
            "plugin name '{APP_ACL_KEY}' is reserved for app capabilities"
        ));
    }
    let parsed = PluginName::try_new(plugin_name)
        .map_err(|error| format!("invalid plugin name '{plugin_name}': {error}"))?;
    if parsed.as_ref() != plugin_name {
        return Err(format!(
            "invalid plugin name '{plugin_name}': expected canonical lowercase kebab-case '{}'",
            parsed.as_ref()
        ));
    }
    Ok(parsed)
}

fn parse_plugin_command_key(command: &str) -> Option<(&str, &str)> {
    command.strip_prefix(PLUGIN_KEY_PREFIX)?.split_once('|')
}

fn command_permission_suffix(command: &str) -> String {
    let mut out = String::with_capacity(command.len() + 4);
    for (index, ch) in command.chars().enumerate() {
        if ch == '_' {
            if !out.ends_with('-') {
                out.push('-');
            }
        } else if ch.is_ascii_uppercase() {
            if index > 0 && !out.ends_with('-') {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
