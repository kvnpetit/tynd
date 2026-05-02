//! Granular ACL / capabilities layer built on `tauri_utils::acl`.
//!
//! ## Hybrid model — `security.rs` vs `acl.rs`
//!
//! The host runs **two** allow-list layers, each tuned for a different job:
//!
//! - [`os::security`](super::security) — the legacy fast path. Holds glob
//!   allow/deny lists for `fs.*` and `http.*` and is consulted on every
//!   filesystem / network call. The check is a single `RwLock::read` plus
//!   a `globset` match — sub-microsecond, suitable for the inner loop.
//!
//! - **`os::acl`** (this module) — the structured per-window/per-webview/
//!   per-origin gate, modelled on Tauri's
//!   [Capability](https://tauri.app/security/capabilities/) format. Newer
//!   APIs — `vault`, `biometric`, `screenshot`, plugin commands — funnel
//!   through here so an admin can write a JSON manifest like:
//!
//!   ```json
//!   {
//!     "identifier": "main-window-default",
//!     "description": "Default permissions for the main window.",
//!     "permissions": ["fs:allow-app-read", "vault:allow-all"],
//!     "windows": ["main"],
//!     "platforms": ["macOS", "windows"]
//!   }
//!   ```
//!
//!   …and have it enforced uniformly. Per-window means the `settings`
//!   window can be locked down even when the `main` window has full
//!   `vault` access; per-platform means `biometric:authenticate` only
//!   resolves on macOS even if every other capability is identical.
//!
//! This module is purely **additive** — `security.rs` keeps owning the
//! fs/http hot path. New code that wants per-context granularity calls
//! [`authority`]`().is_allowed(...)` instead.
//!
//! ## API surface
//!
//! The JS frontend talks to this module via the `acl.*` dispatch namespace:
//!
//! - `acl.load(manifestPath)` — load capabilities from a `.json` file
//!   (single object, array, or `{ "capabilities": [...] }`).
//! - `acl.addCapability(cap)` / `acl.removeCapability(id)` — runtime
//!   mutation. Triggers a rebuild of the resolved lookup table.
//! - `acl.isAllowed({ command, window, webview, origin })` — gate query.
//! - `acl.list()` — inspection / debug.
//! - `acl.registerManifest(pluginName, manifest)` — only used internally
//!   by plugins as they boot; not exposed from the dispatch table.
//!
//! Commands are matched in the format `plugin:command` (e.g. `fs:read`,
//! `vault:write`) — the resolver maps that to the internal
//! `plugin:fs|read` shape produced by `tauri_utils::acl::resolved`.
//! Permissions written without a prefix (`open`) target the **app**
//! manifest at the [`APP_ACL_KEY`] slot.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

use parking_lot::RwLock;
use serde_json::{json, Value};
use tauri_utils::{
    acl::{
        capability::{Capability, CapabilityFile},
        manifest::Manifest,
        resolved::Resolved,
        APP_ACL_KEY,
    },
    platform::Target,
};

/// Singleton authority: holds capabilities + plugin manifests + a
/// pre-resolved lookup table. Rebuilt whenever capabilities or manifests
/// change so `is_allowed` stays a single `BTreeMap::get` + glob-match.
#[derive(Debug)]
pub struct Authority {
    /// Active capabilities, keyed by `Capability::identifier`.
    capabilities: RwLock<BTreeMap<String, Capability>>,
    /// Plugin manifests, keyed by plugin name (or [`APP_ACL_KEY`] for
    /// app-level permissions). One entry per plugin that declares
    /// permissions — the resolver walks this map to expand permission
    /// references into concrete commands.
    manifests: RwLock<BTreeMap<String, Manifest>>,
    /// Compiled lookup. `None` until the first capability is loaded
    /// (matches the "open by default" behaviour of [`os::security`] —
    /// without an active manifest, every call is allowed). Rebuilt on
    /// every mutation.
    resolved: RwLock<Option<Resolved>>,
    /// Sticky latch — flips to `true` the first time `add_capability`
    /// (or `load_file`) registers a real grant and **never** flips
    /// back. Used by `is_allowed` to keep the deny-by-default posture
    /// after an admin removes the last capability: without this an
    /// attacker (or accidental cleanup) could revert a configured
    /// process to the open-by-default mode of an unconfigured one.
    ever_configured: AtomicBool,
}

impl Authority {
    fn new() -> Self {
        Self {
            capabilities: RwLock::new(BTreeMap::new()),
            manifests: RwLock::new(BTreeMap::new()),
            resolved: RwLock::new(None),
            ever_configured: AtomicBool::new(false),
        }
    }

    /// Load capabilities from a `.json` capability file (single object,
    /// list, or named-list shape — all three are accepted by
    /// [`CapabilityFile`]). Adds them to the in-memory set; existing
    /// capabilities with the same identifier are replaced.
    pub fn load_file(&self, path: &Path) -> Result<(), String> {
        let file = CapabilityFile::load(path)
            .map_err(|e| format!("acl.load: failed to read '{}': {e}", path.display()))?;
        let caps: Vec<Capability> = match file {
            CapabilityFile::Capability(c) => vec![c],
            CapabilityFile::List(list) | CapabilityFile::NamedList { capabilities: list } => list,
        };
        let inserted_any = !caps.is_empty();
        {
            let mut write = self.capabilities.write();
            for c in caps {
                write.insert(c.identifier.clone(), c);
            }
        }
        if inserted_any {
            // Sticky — we now have at least one grant on file, so the
            // process is in "configured" mode for the rest of its life.
            self.ever_configured.store(true, Ordering::SeqCst);
        }
        self.rebuild_resolved()
    }

    /// Add a capability at runtime. Replaces any existing capability
    /// with the same identifier.
    pub fn add_capability(&self, cap: Capability) -> Result<(), String> {
        self.capabilities.write().insert(cap.identifier.clone(), cap);
        // Sticky latch — see `Authority::ever_configured`. Stays true
        // even after a subsequent `remove_capability` empties the map.
        self.ever_configured.store(true, Ordering::SeqCst);
        self.rebuild_resolved()
    }

    /// Remove a capability by identifier. No-op if not present.
    pub fn remove_capability(&self, id: &str) -> Result<(), String> {
        self.capabilities.write().remove(id);
        self.rebuild_resolved()
    }

    /// Register a plugin's manifest (one per plugin). Pass
    /// [`APP_ACL_KEY`] as `name` to register the app-level manifest.
    /// Triggers a rebuild — call once per plugin at boot, not in a hot
    /// loop.
    pub fn register_manifest(&self, name: &str, manifest: Manifest) -> Result<(), String> {
        self.manifests.write().insert(name.to_string(), manifest);
        self.rebuild_resolved()
    }

    /// All currently-loaded capabilities (for inspection / debug).
    pub fn list(&self) -> Vec<Capability> {
        self.capabilities.read().values().cloned().collect()
    }

    /// Check if a command is allowed for the given context.
    ///
    /// `command` is a user-friendly identifier — `"fs:read"`,
    /// `"vault:allow-all"`, or just `"some-app-command"` for the app
    /// manifest. Internally rewritten to the resolved shape
    /// (`plugin:fs|read`) before lookup.
    ///
    /// `window` / `webview` are matched against the `windows` /
    /// `webviews` glob lists declared on each capability. A `None` value
    /// is treated as "any" — useful from background tasks that aren't
    /// tied to a specific window.
    ///
    /// `origin` is the URL of the caller; capabilities with a `remote`
    /// block only match when `origin` parses to a URL covered by the
    /// pattern. A `None` origin is treated as the local app context
    /// (the `local: true` default on every capability).
    ///
    /// **Default:** when no capabilities are loaded, returns `true`
    /// (open by default — same posture as `os::security`).
    pub fn is_allowed(
        &self,
        command: &str,
        window: Option<&str>,
        webview: Option<&str>,
        origin: Option<&str>,
    ) -> bool {
        let resolved_guard = self.resolved.read();
        let Some(resolved) = resolved_guard.as_ref() else {
            // Nothing resolved. Two cases:
            //  * Truly unconfigured (no `add_capability` / `load_file`
            //    has ever run) → fail open, matching the `security.rs`
            //    posture before `security.configure` is called.
            //  * Was configured, all capabilities later removed → fail
            //    closed. Once an admin has expressed intent to gate
            //    this process, an empty list means "deny everything",
            //    not "back to no policy". The sticky `ever_configured`
            //    latch encodes that.
            return !self.ever_configured.load(Ordering::SeqCst);
        };

        let lookup_keys = command_lookup_keys(command);

        // Deny precedence: any matching deny rule wins.
        for key in &lookup_keys {
            if let Some(denied) = resolved.denied_commands.get(key.as_str()) {
                if denied
                    .iter()
                    .any(|cmd| context_matches(cmd, window, webview, origin))
                {
                    return false;
                }
            }
        }

        for key in &lookup_keys {
            if let Some(allowed) = resolved.allowed_commands.get(key.as_str()) {
                if allowed
                    .iter()
                    .any(|cmd| context_matches(cmd, window, webview, origin))
                {
                    return true;
                }
            }
        }

        // No matching allow rule and no matching deny rule. Once at
        // least one capability is active, the contract flips to
        // "deny by default" — the caller has opted into the ACL model.
        false
    }

    /// Recompile the resolved lookup. Cheap (a few μs for typical
    /// manifests); runs under a write lock on `resolved` so callers
    /// reading mid-rebuild see the previous state.
    fn rebuild_resolved(&self) -> Result<(), String> {
        let caps = self.capabilities.read().clone();
        let mans = self.manifests.read();

        if caps.is_empty() {
            *self.resolved.write() = None;
            return Ok(());
        }

        let resolved = Resolved::resolve(&mans, caps, Target::current())
            .map_err(|e| format!("acl: resolve failed — {e}"))?;
        *self.resolved.write() = Some(resolved);
        Ok(())
    }
}

/// Singleton accessor — same pattern as `os::security`.
static AUTHORITY: OnceLock<Authority> = OnceLock::new();

/// Get (or lazily create) the process-wide ACL authority.
pub fn authority() -> &'static Authority {
    AUTHORITY.get_or_init(Authority::new)
}

/// Build the set of resolved-command keys to look up for a user-friendly
/// `command` string.
///
/// `tauri_utils::acl::resolved` stores keys in the form
/// `plugin:{name}|{cmd}` for plugin commands and `{cmd}` for app-level
/// commands. Callers pass the friendlier `name:cmd`; we translate.
fn command_lookup_keys(command: &str) -> Vec<String> {
    if let Some((plugin, cmd)) = command.split_once(':') {
        // Try both forms — some manifests register against the plugin
        // prefix, others (app-level) leave the colon as a literal.
        vec![
            format!("plugin:{plugin}|{cmd}"),
            command.to_string(),
        ]
    } else {
        vec![command.to_string()]
    }
}

/// Match a single resolved command's context against the caller's
/// window / webview / origin tuple.
fn context_matches(
    cmd: &tauri_utils::acl::resolved::ResolvedCommand,
    window: Option<&str>,
    webview: Option<&str>,
    origin: Option<&str>,
) -> bool {
    use tauri_utils::acl::ExecutionContext;

    // Origin gate first — `local` (default) capabilities accept any
    // None / `tauri://` style origin; a `remote` capability only
    // matches when its URL pattern accepts the caller's URL.
    let origin_ok = match (&cmd.context, origin) {
        (ExecutionContext::Local, _) => true,
        (ExecutionContext::Remote { url }, Some(o)) => o
            .parse::<url::Url>()
            .map(|parsed| url.test(&parsed))
            .unwrap_or(false),
        (ExecutionContext::Remote { .. }, None) => false,
    };
    if !origin_ok {
        return false;
    }

    // Window match — empty list means "any window".
    let window_ok = cmd.windows.is_empty()
        || window
            .map(|w| cmd.windows.iter().any(|p| p.matches(w)))
            .unwrap_or(true);

    // Webview match — same rule.
    let webview_ok = cmd.webviews.is_empty()
        || webview
            .map(|wv| cmd.webviews.iter().any(|p| p.matches(wv)))
            .unwrap_or(true);

    // A capability with **both** windows and webviews populated grants
    // access when *either* axis matches (mirrors Tauri's runtime gate
    // — see `Capability` doc on `webviews`). When only one axis is set,
    // the other is "any".
    if !cmd.windows.is_empty() && !cmd.webviews.is_empty() {
        window_ok || webview_ok
    } else {
        window_ok && webview_ok
    }
}

/// Generate a JSON-Schema describing the [`Capability`] format and
/// write it to `out_path`. Consumed by IDE autocomplete on
/// `*.tynd-capability.json` files.
///
/// Hand-rolled rather than relying on `schemars` (`tauri_utils` only
/// derives `JsonSchema` behind its own `schema` feature, and exposing
/// `schemars` to consumers would force another workspace dep). The
/// shape mirrors the `Capability` struct documented at
/// <https://tauri.app/security/capabilities/>.
pub fn generate_schema(out_path: &Path) -> Result<(), String> {
    let schema = capability_schema();
    let serialized = serde_json::to_string_pretty(&schema)
        .map_err(|e| format!("acl.schema: serialize failed — {e}"))?;
    std::fs::write(out_path, serialized)
        .map_err(|e| format!("acl.schema: write '{}': {e}", out_path.display()))
}

/// Build the JSON Schema document for a single capability file.
fn capability_schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Tynd Capability",
        "description": "Permission grant for one or more windows / webviews. \
                        See https://tauri.app/security/capabilities/ for the \
                        underlying tauri-utils format.",
        "oneOf": [
            { "$ref": "#/definitions/Capability" },
            {
                "type": "array",
                "items": { "$ref": "#/definitions/Capability" }
            },
            {
                "type": "object",
                "required": ["capabilities"],
                "properties": {
                    "capabilities": {
                        "type": "array",
                        "items": { "$ref": "#/definitions/Capability" }
                    }
                }
            }
        ],
        "definitions": {
            "Capability": {
                "type": "object",
                "required": ["identifier", "permissions"],
                "properties": {
                    "identifier": {
                        "type": "string",
                        "description": "Unique capability id (e.g. main-window-default)."
                    },
                    "description": { "type": "string" },
                    "local": {
                        "type": "boolean",
                        "default": true,
                        "description": "Whether this capability applies to local app URLs."
                    },
                    "remote": {
                        "type": "object",
                        "required": ["urls"],
                        "properties": {
                            "urls": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "URL patterns that may use this capability \
                                                (https://urlpattern.spec.whatwg.org/)."
                            }
                        }
                    },
                    "windows": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Window labels (glob patterns supported)."
                    },
                    "webviews": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Webview labels (glob patterns supported)."
                    },
                    "permissions": {
                        "type": "array",
                        "uniqueItems": true,
                        "items": {
                            "oneOf": [
                                { "type": "string" },
                                {
                                    "type": "object",
                                    "required": ["identifier"],
                                    "properties": {
                                        "identifier": { "type": "string" },
                                        "allow": { "type": "array" },
                                        "deny": { "type": "array" }
                                    }
                                }
                            ]
                        }
                    },
                    "platforms": {
                        "type": "array",
                        "items": {
                            "enum": ["macOS", "windows", "linux", "android", "iOS"]
                        }
                    }
                }
            }
        }
    })
}

/// Dispatch `acl.*` IPC calls from the JS bridge. Mirrors the shape of
/// every other `os::*` dispatch fn — `Result<Value, String>` so the
/// frontend gets a textual error.
pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "load" => {
            let path = args
                .get("manifestPath")
                .or_else(|| args.get("path"))
                .and_then(Value::as_str)
                .ok_or_else(|| "acl.load: missing 'manifestPath'".to_string())?;
            authority().load_file(Path::new(path))?;
            Ok(Value::Null)
        },
        "addCapability" => {
            let cap_value = args
                .get("capability")
                .cloned()
                .unwrap_or_else(|| args.clone());
            let cap: Capability = serde_json::from_value(cap_value)
                .map_err(|e| format!("acl.addCapability: invalid shape — {e}"))?;
            authority().add_capability(cap)?;
            Ok(Value::Null)
        },
        "removeCapability" => {
            let id = args
                .get("identifier")
                .or_else(|| args.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(|| "acl.removeCapability: missing 'identifier'".to_string())?;
            authority().remove_capability(id)?;
            Ok(Value::Null)
        },
        "isAllowed" => {
            let command = args
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| "acl.isAllowed: missing 'command'".to_string())?;
            let window = args.get("window").and_then(Value::as_str);
            let webview = args.get("webview").and_then(Value::as_str);
            let origin = args.get("origin").and_then(Value::as_str);
            Ok(Value::Bool(
                authority().is_allowed(command, window, webview, origin),
            ))
        },
        "list" => serde_json::to_value(authority().list())
            .map_err(|e| format!("acl.list: serialize failed — {e}")),
        "generateSchema" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "acl.generateSchema: missing 'path'".to_string())?;
            generate_schema(Path::new(path))?;
            Ok(Value::Null)
        },
        _ => Err(format!("acl.{method}: unknown method")),
    }
}

// Avoid an "unused" warning on the always-on `APP_ACL_KEY` re-export
// when consumers haven't yet wired the app manifest path.
#[allow(dead_code)]
pub(crate) const _APP_ACL_KEY: &str = APP_ACL_KEY;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::num::NonZeroU64;
    use tauri_utils::acl::{
        capability::{Capability, PermissionEntry},
        manifest::Manifest,
        Commands, Permission,
    };

    /// Tests share the global `AUTHORITY` singleton; serialize them so
    /// one test's capabilities don't leak into another.
    fn test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        &LOCK
    }

    /// Wipe the singleton's state. Cheaper than rebuilding the whole
    /// `OnceLock` (which we can't anyway). Also clears the sticky
    /// `ever_configured` latch so each test starts from a clean
    /// "unconfigured" slate — production code never resets the latch
    /// on purpose, but tests own the singleton across runs.
    fn reset() {
        let auth = authority();
        auth.capabilities.write().clear();
        auth.manifests.write().clear();
        *auth.resolved.write() = None;
        auth.ever_configured.store(false, Ordering::SeqCst);
    }

    /// Build a minimal `Manifest` with one named permission whose
    /// `commands.allow` lists a single command.
    fn manifest_with(perm_id: &str, allow_cmd: &str) -> Manifest {
        let mut permissions = std::collections::BTreeMap::new();
        permissions.insert(
            perm_id.to_string(),
            Permission {
                version: NonZeroU64::new(1),
                identifier: perm_id.to_string(),
                description: None,
                commands: Commands {
                    allow: vec![allow_cmd.to_string()],
                    deny: vec![],
                },
                scope: Default::default(),
                platforms: None,
            },
        );
        Manifest {
            default_permission: None,
            permissions,
            permission_sets: std::collections::BTreeMap::new(),
            global_scope_schema: None,
        }
    }

    fn capability(id: &str, plugin_perm: &str, windows: Vec<&str>) -> Capability {
        let identifier =
            tauri_utils::acl::Identifier::try_from(plugin_perm.to_string()).expect("ident");
        Capability {
            identifier: id.to_string(),
            description: String::new(),
            remote: None,
            local: true,
            windows: windows.into_iter().map(String::from).collect(),
            webviews: vec![],
            permissions: vec![PermissionEntry::PermissionRef(identifier)],
            platforms: None,
        }
    }

    #[test]
    fn unconfigured_authority_allows_everything() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        // No manifests, no capabilities → open by default. Mirrors the
        // `security.rs` behaviour before `security.configure` is called.
        assert!(authority().is_allowed("vault:read", Some("main"), None, None));
        assert!(authority().is_allowed("anything-else", None, None, None));
    }

    #[test]
    fn add_capability_grants_referenced_command() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");
        authority()
            .add_capability(capability("c1", "vault:allow-all", vec![]))
            .expect("add");
        assert!(authority().is_allowed("vault:read", Some("main"), None, None));
        // Unknown command stays denied.
        assert!(!authority().is_allowed("vault:write", Some("main"), None, None));
    }

    #[test]
    fn window_glob_filters_grant() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("fs", manifest_with("allow-read", "read"))
            .expect("register");
        authority()
            .add_capability(capability("main-only", "fs:allow-read", vec!["main"]))
            .expect("add");
        assert!(
            authority().is_allowed("fs:read", Some("main"), None, None),
            "main window should match"
        );
        assert!(
            !authority().is_allowed("fs:read", Some("settings"), None, None),
            "settings window should NOT match"
        );
    }

    #[test]
    fn platform_filter_excludes_other_oses() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("biometric", manifest_with("allow-auth", "authenticate"))
            .expect("register");

        // Pick an OS that's NOT the current one. Resolved::resolve
        // skips capabilities whose `platforms` doesn't include the
        // build target, so the resulting `is_allowed` must be false.
        let other = if cfg!(target_os = "macos") {
            Target::Linux
        } else {
            Target::MacOS
        };
        let mut cap = capability("biometric-other", "biometric:allow-auth", vec![]);
        cap.platforms = Some(vec![other]);
        authority().add_capability(cap).expect("add");

        assert!(!authority().is_allowed("biometric:authenticate", None, None, None));
    }

    #[test]
    fn remove_capability_revokes_grant() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");
        authority()
            .add_capability(capability("c1", "vault:allow-all", vec![]))
            .expect("add");
        assert!(authority().is_allowed("vault:read", None, None, None));

        authority().remove_capability("c1").expect("remove");
        // Removing the last capability does NOT reopen the gate — once
        // the authority has ever been configured, an empty capability
        // set means deny-by-default. Otherwise an admin clearing the
        // policy by mistake (or an attacker via the runtime API) would
        // hand the process back to the open-by-default mode of an
        // unconfigured one. See `Authority::ever_configured`.
        assert!(!authority().is_allowed("vault:read", None, None, None));
    }

    #[test]
    fn ever_configured_latch_persists_across_full_clear() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");
        authority()
            .add_capability(capability("c1", "vault:allow-all", vec![]))
            .expect("add");

        // Drop every capability — production callers can't unset the
        // latch, so we go through the public API and confirm the gate
        // stays closed for both known and unknown commands.
        authority().remove_capability("c1").expect("remove");
        assert!(!authority().is_allowed("vault:read", None, None, None));
        assert!(!authority().is_allowed("anything-else", None, None, None));
    }

    #[test]
    fn unknown_plugin_manifest_denies_silently() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        // Register a manifest for plugin `vault` only.
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");
        authority()
            .add_capability(capability("c1", "vault:allow-all", vec![]))
            .expect("add");

        // `unknown:foo` references a plugin the resolver has never heard of.
        // Capability resolution will fail for the unrelated capability we
        // try to add, but the existing one stays valid — and the unknown
        // command is denied by absence.
        assert!(!authority().is_allowed("unknown:foo", None, None, None));
    }

    #[test]
    fn list_returns_loaded_capabilities() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");
        authority()
            .add_capability(capability("c1", "vault:allow-all", vec!["main"]))
            .expect("add");
        let listed = authority().list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].identifier, "c1");
    }

    #[test]
    fn dispatch_round_trip() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");

        // addCapability via dispatch
        let cap_json = json!({
            "capability": {
                "identifier": "dispatch-cap",
                "description": "",
                "permissions": ["vault:allow-all"],
                "windows": ["main"]
            }
        });
        dispatch("addCapability", &cap_json).expect("addCapability");

        // isAllowed via dispatch
        let res = dispatch(
            "isAllowed",
            &json!({ "command": "vault:read", "window": "main" }),
        )
        .expect("isAllowed");
        assert_eq!(res, Value::Bool(true));

        // list via dispatch
        let list_res = dispatch("list", &Value::Null).expect("list");
        assert_eq!(list_res.as_array().map(Vec::len), Some(1));

        // removeCapability via dispatch
        dispatch(
            "removeCapability",
            &json!({ "identifier": "dispatch-cap" }),
        )
        .expect("removeCapability");
        let list_after = dispatch("list", &Value::Null).expect("list");
        assert_eq!(list_after.as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn load_file_reads_capability_json() {
        let _g = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        reset();
        authority()
            .register_manifest("vault", manifest_with("allow-all", "read"))
            .expect("register");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cap.json");
        std::fs::write(
            &path,
            r#"{
                "identifier": "from-file",
                "description": "",
                "permissions": ["vault:allow-all"],
                "windows": ["main"]
            }"#,
        )
        .expect("write");

        authority().load_file(&path).expect("load_file");
        let listed = authority().list();
        assert!(listed.iter().any(|c| c.identifier == "from-file"));
    }

    #[test]
    fn generate_schema_writes_valid_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("schema.json");
        generate_schema(&path).expect("generate_schema");
        let content = std::fs::read_to_string(&path).expect("read");
        let parsed: Value = serde_json::from_str(&content).expect("parse");
        assert_eq!(parsed["title"], "Tynd Capability");
        assert!(parsed["definitions"]["Capability"].is_object());
    }
}
