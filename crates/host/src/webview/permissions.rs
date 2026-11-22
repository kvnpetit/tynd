//! Web permission policy (`window.permissions` / `webview.create({ permissions })`).
//!
//! wry calls the handler synchronously on the UI thread with only the
//! permission kind, so the decision is a static per-kind policy combined with
//! what the webview currently shows: app content keeps the platform flow for
//! kinds the app did not configure, remote content is denied.

use std::collections::BTreeMap;

use serde_json::Value;
use wry::{PermissionKind, PermissionResponse};

use super::handlers::is_app_local_webview;

/// Decision configured for one permission kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionDecision {
    Allow,
    Deny,
    /// Platform flow: the WebView's own prompt (WebView2, WKWebView). WebKitGTK
    /// has no prompt and denies.
    Prompt,
}

impl PermissionDecision {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }

    fn to_wry(self) -> PermissionResponse {
        match self {
            Self::Allow => PermissionResponse::Allow,
            Self::Deny => PermissionResponse::Deny,
            Self::Prompt => PermissionResponse::Default,
        }
    }
}

/// Config keys, in the order of the docs table.
const KINDS: &[(&str, PermissionKind)] = &[
    ("camera", PermissionKind::Camera),
    ("microphone", PermissionKind::Microphone),
    ("geolocation", PermissionKind::Geolocation),
    ("notifications", PermissionKind::Notifications),
    ("clipboardRead", PermissionKind::ClipboardRead),
    ("displayCapture", PermissionKind::DisplayCapture),
    ("midi", PermissionKind::Midi),
    ("sensors", PermissionKind::Sensors),
    ("localFonts", PermissionKind::LocalFonts),
    ("windowManagement", PermissionKind::WindowManagement),
    ("pointerLock", PermissionKind::PointerLock),
    ("automaticDownloads", PermissionKind::AutomaticDownloads),
    ("fileSystemAccess", PermissionKind::FileSystemAccess),
    ("autoplay", PermissionKind::Autoplay),
];

/// Per-kind decisions. Kinds left unset use [`PermissionPolicy::decide`]'s
/// content-dependent default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PermissionPolicy {
    decisions: Vec<(&'static str, PermissionDecision)>,
}

impl PermissionPolicy {
    /// Parse `{ camera: "allow", ... }` from `window.create` /
    /// `webview.create` arguments. `null` / absent = default policy.
    pub(crate) fn from_value(raw: Option<&Value>, operation: &str) -> Result<Self, String> {
        match raw {
            None | Some(Value::Null) => Ok(Self::default()),
            Some(Value::Object(map)) => Self::from_entries(
                map.iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
                operation,
            ),
            Some(_) => Err(format!("{operation}: permissions must be an object")),
        }
    }

    /// Parse the primary window's `permissions` config.
    pub(crate) fn from_map(
        raw: Option<&BTreeMap<String, String>>,
        operation: &str,
    ) -> Result<Self, String> {
        raw.map_or_else(
            || Ok(Self::default()),
            |map| {
                Self::from_entries(
                    map.iter()
                        .map(|(key, value)| (key.as_str(), Some(value.as_str()))),
                    operation,
                )
            },
        )
    }

    fn from_entries<'a>(
        entries: impl Iterator<Item = (&'a str, Option<&'a str>)>,
        operation: &str,
    ) -> Result<Self, String> {
        let mut decisions = Vec::new();
        for (key, value) in entries {
            let Some(&(name, _)) = KINDS.iter().find(|(name, _)| *name == key) else {
                let known: Vec<&str> = KINDS.iter().map(|(name, _)| *name).collect();
                return Err(format!(
                    "{operation}: unknown permission '{key}' (expected one of {})",
                    known.join(", ")
                ));
            };
            let decision = value.and_then(PermissionDecision::parse).ok_or_else(|| {
                format!("{operation}: permissions.{key} must be \"allow\", \"deny\" or \"prompt\"")
            })?;
            decisions.push((name, decision));
        }
        Ok(Self { decisions })
    }

    fn configured(&self, kind: PermissionKind) -> Option<PermissionDecision> {
        let name = KINDS
            .iter()
            .find(|(_, candidate)| *candidate == kind)
            .map(|(name, _)| *name)?;
        self.decisions
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, decision)| *decision)
    }

    /// Decision for `kind` while the webview shows app content (`app_local`)
    /// or remote content. A configured kind applies to both; an unset kind
    /// keeps the platform flow for app content and is denied for remote
    /// content.
    pub(crate) fn decide(&self, kind: PermissionKind, app_local: bool) -> PermissionDecision {
        self.configured(kind).unwrap_or(if app_local {
            PermissionDecision::Prompt
        } else {
            PermissionDecision::Deny
        })
    }
}

/// wry permission handler for the webview labelled `label`.
pub(crate) fn permission_handler(
    label: &str,
    policy: PermissionPolicy,
) -> impl Fn(PermissionKind) -> PermissionResponse + Send + Sync + 'static {
    let label = label.to_owned();
    move |kind| {
        let decision = policy.decide(kind, is_app_local_webview(&label));
        if decision == PermissionDecision::Deny {
            crate::tynd_debug!("webview '{label}': {kind} permission denied by policy");
        }
        decision.to_wry()
    }
}

#[cfg(test)]
mod tests;
