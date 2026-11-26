//! Granular ACL / capabilities layer built on Tynd's internal resolver.
//!
//! ## Hybrid model — `security.rs` vs `acl.rs`
//!
//! The host runs **two** allow-list layers, each tuned for a different job:
//!
//! - [`crate::security::policy`](crate::security::policy) — the legacy fast path. Holds glob allow/deny lists for
//!   `fs.*` and `http.*` and is consulted on every filesystem / network call. The check is a single
//!   `RwLock::read` plus a `globset` match — sub-microsecond, suitable for the inner loop.
//!
//! - **`crate::security::acl`** (this module) — the structured per-window/per-webview/ per-origin gate, modelled
//!   on Tynd capability files. Newer APIs — `vault`, `biometric`, `screenshot`, plugin commands —
//!   funnel through here so an admin can write a JSON manifest like:
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
//! Both layers must allow a renderer request: this authority grants the
//! command/context first, then `security.rs` applies resource scopes such as
//! filesystem paths and URLs.
//!
//! ## API surface
//!
//! The `acl.*` dispatch namespace cannot change capabilities. They are
//! declarative and arrive once in the backend startup frame
//! ([`Authority::load_value`]).
//!
//! - `acl.isAllowed({ command, window, webview, origin })` — gate query.
//! - `acl.list()` — inspection / debug.
//! - `acl.generateSchema({ path })` — write the capability JSON-Schema.
//!
//! Plugins register their manifests from Rust at boot through
//! [`Authority::register_manifest`]; that is not exposed over IPC.
//!
//! Commands are matched in the format `plugin:command` (e.g. `fs:read`,
//! `vault:write`); the resolver maps that to its internal `plugin:fs|read`
//! shape. Permissions written without a prefix (`open`) target the **app**
//! manifest at the [`APP_ACL_KEY`](tynd_capabilities::APP_ACL_KEY) slot.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, LazyLock},
};

use arc_swap::ArcSwap;
use parking_lot::Mutex;
use serde_json::Value;
use tynd_capabilities::{
    capability_schema, validate_capability_shape, Capability, CapabilityFile, Commands, Manifest,
    Permission, PermissionEntry, Resolved, Target,
};

use crate::api::HostApi;
use crate::ipc::error::IpcError;

/// Prefix of the `PermissionRef` / command namespace of user backend functions.
const BACKEND_PREFIX: &str = "backend:";

/// Authority posture — single enum that encodes both "has the deployer
/// turned the gate on?" and "what's the resolved lookup table?", so a
/// reader can't observe a partial transition between the two.
#[derive(Debug, Clone, Default)]
enum Posture {
    /// No grant exists. This is both the startup posture and the state
    /// reached after the final capability is removed.
    #[default]
    DenyAll,
    /// At least one capability is live; `is_allowed` walks the
    /// `Resolved` table to decide.
    Configured(Configured),
}

/// Resolved lookup table plus the command namespaces (`backend:`, `window:`…)
/// that the loaded capabilities mention at all, computed once at load.
#[derive(Debug, Clone)]
struct Configured {
    resolved: Arc<Resolved>,
    /// `api:` prefix (up to and including the first `:`) of every allow/deny
    /// command and permission reference, regardless of platform.
    declared_prefixes: Arc<BTreeSet<String>>,
}

/// Immutable snapshot of the authority state. Lives behind an `ArcSwap`
/// so `is_allowed` reads are lock-free — single atomic load + Arc bump
/// per call. The `Resolved` lookup table is the only part the hot path
/// reads; capabilities live here too so `Authority::list()` doesn't
/// have to go through the manifest mutex. `Resolved` is wrapped in `Arc`
/// so cloning the snapshot only bumps a refcount.
#[derive(Debug, Clone, Default)]
struct AuthorityState {
    /// Active capabilities, keyed by `Capability::identifier`.
    capabilities: BTreeMap<String, Capability>,
    /// Posture flag + compiled lookup, fused into one field so readers
    /// can't observe a half-applied transition between "configured" and
    /// "resolved".
    posture: Posture,
    /// Set when a capability manifest failed to load. Sticky until a later
    /// load succeeds: while set, every check denies, including the opt-in
    /// namespaces (`backend:`, `window:`…) that otherwise allow when undeclared.
    fail_closed: bool,
}

/// Singleton authority: holds capabilities + a pre-resolved lookup
/// table behind an `ArcSwap` for lock-free reads. Rebuilt whenever
/// capabilities or manifests change so `is_allowed` stays a single
/// `BTreeMap::get` + glob-match on a stable snapshot.
#[derive(Debug)]
pub(crate) struct Authority {
    /// Lock-free read state. Snapshots are immutable once published —
    /// `is_allowed` only does `state.load()` (atomic) + Arc deref.
    state: ArcSwap<AuthorityState>,
    /// Plugin manifests, outside the snapshot: writes are rare (one
    /// `register_manifest` per plugin at boot), so this mutex never blocks
    /// the hot path. The `Resolved` rebuild reads this map while holding
    /// `write_lock`, so a manifest swap can't race with a capability add.
    manifests: Mutex<BTreeMap<String, Manifest>>,
    /// Writer-only mutex. Serializes RMW (`register_manifest`,
    /// `load_value`) so two concurrent mutators don't lose updates between
    /// `state.load_full()` and `state.store(Arc::new(...))`. Held only
    /// on the slow path; readers never touch it.
    write_lock: Mutex<()>,
}

impl Authority {
    fn new() -> Self {
        Self {
            state: ArcSwap::from_pointee(AuthorityState::default()),
            manifests: Mutex::new(BTreeMap::new()),
            write_lock: Mutex::new(()),
        }
    }

    /// RMW under the writer mutex: clone the current snapshot, hand it
    /// to `f`, recompute `resolved` from the mutated capabilities +
    /// the current manifest set, and publish the new `Arc`. The lock
    /// is dropped after `store(...)` so a follow-up reader sees the
    /// new state on its next `state.load()`.
    fn mutate_state<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce(&mut AuthorityState) -> R,
    {
        let _guard = self.write_lock.lock();
        let current = self.state.load_full();
        let mut next = (*current).clone();
        let r = f(&mut next);
        next.posture = self.recompute_posture(&next.capabilities)?;
        self.state.store(Arc::new(next));
        Ok(r)
    }

    /// Re-resolve the lookup table from the current capabilities +
    /// manifests. Caller holds `write_lock` so the manifest snapshot
    /// is consistent with the capabilities being resolved against.
    fn recompute_posture(
        &self,
        capabilities: &BTreeMap<String, Capability>,
    ) -> Result<Posture, String> {
        let mans = self.manifests.lock();
        Self::recompute_posture_with(&mans, capabilities)
    }

    fn recompute_posture_with(
        manifests: &BTreeMap<String, Manifest>,
        capabilities: &BTreeMap<String, Capability>,
    ) -> Result<Posture, String> {
        if capabilities.is_empty() {
            return Ok(Posture::DenyAll);
        }
        let normalized: BTreeMap<String, Capability> = capabilities
            .iter()
            .map(|(id, capability)| (id.clone(), normalize_backend_refs(capability)))
            .collect();
        let resolved = Resolved::resolve(manifests, normalized, Target::current())
            .map_err(|e| format!("acl: resolve failed — {e}"))?;
        Ok(Posture::Configured(Configured {
            resolved: Arc::new(resolved),
            declared_prefixes: Arc::new(declared_prefixes(capabilities)),
        }))
    }

    /// Load a capability value supplied by the trusted backend startup
    /// frame. Fail-closed: if the value fails to parse / validate, the
    /// authority is forced into `Posture::DenyAll` before the error is
    /// returned, so a corrupt manifest never retains earlier grants.
    pub(crate) fn load_value(&self, value: Value) -> Result<(), String> {
        let source = Path::new("<backend-startup>");
        let file: CapabilityFile = serde_json::from_value(value).map_err(|e| {
            self.trip_fail_closed(source, &format!("parse failed - {e}"));
            IpcError::new(
                "acl",
                "configure",
                format!("invalid startup capabilities: {e} - deny-by-default in effect"),
            )
            .into_string()
        })?;
        let caps = file.into_capabilities();
        for capability in &caps {
            if let Err(error) = validate_capability(capability) {
                self.trip_fail_closed(source, &format!("capability shape invalid - {error}"));
                return Err(error);
            }
        }

        self.mutate_state(|state| {
            state.fail_closed = false;
            for capability in caps {
                state
                    .capabilities
                    .insert(capability.identifier.clone(), capability);
            }
        })
        .map_err(|error| {
            self.trip_fail_closed(source, &format!("resolve failed - {error}"));
            error
        })
    }

    /// Slam the authority into `Posture::DenyAll` after an untrusted
    /// capability-file load failure. Programmatic manifest registration is
    /// transactional instead: a rejected replacement leaves the last valid
    /// manifest and resolved posture untouched.
    fn trip_fail_closed(&self, path: &Path, reason: &str) {
        tracing::error!(
            target: "tynd::host::acl",
            path = %path.display(),
            reason = reason,
            "ACL manifest unloadable; forcing deny-by-default posture"
        );
        let _guard = self.write_lock.lock();
        let mut next = (*self.state.load_full()).clone();
        next.capabilities.clear();
        next.posture = Posture::DenyAll;
        next.fail_closed = true;
        self.state.store(Arc::new(next));
    }

    /// Register a plugin's manifest (one per plugin). Pass
    /// [`APP_ACL_KEY`](tynd_capabilities::APP_ACL_KEY) as `name` to register the app-level manifest.
    /// Triggers a rebuild — call once per plugin at boot, not in a hot
    /// loop.
    #[cfg(feature = "plugins")]
    pub(crate) fn register_manifest(&self, name: &str, manifest: Manifest) -> Result<(), String> {
        // Resolve against a private candidate map first. The live manifest
        // table and authority snapshot commit together only after resolution
        // succeeds, so a bad plugin can neither leave stale metadata nor
        // revoke an unrelated, previously-valid capability.
        let _guard = self.write_lock.lock();
        let mut manifests = self.manifests.lock();
        let mut next_manifests = manifests.clone();
        next_manifests.insert(name.to_string(), manifest);
        let current = self.state.load_full();
        let mut next = (*current).clone();
        next.posture = Self::recompute_posture_with(&next_manifests, &next.capabilities)
            .map_err(|error| format!("acl: register_manifest('{name}') failed — {error}"))?;
        *manifests = next_manifests;
        self.state.store(Arc::new(next));
        Ok(())
    }

    /// All currently-loaded capabilities (for inspection / debug).
    pub(crate) fn list(&self) -> Vec<Capability> {
        self.state.load().capabilities.values().cloned().collect()
    }

    /// Whether a capability manifest failed to load and the authority is
    /// denying everything until a later load succeeds.
    pub(crate) fn is_fail_closed(&self) -> bool {
        self.state.load().fail_closed
    }

    /// Whether any loaded capability mentions a command or permission whose
    /// `api:` prefix starts with `prefix` (e.g. `"backend:"`). Computed when
    /// the capabilities are loaded, so this is a cheap read on the hot path.
    pub(crate) fn declares_command_prefix(&self, prefix: &str) -> bool {
        match &self.state.load().posture {
            Posture::DenyAll => false,
            Posture::Configured(configured) => configured
                .declared_prefixes
                .iter()
                .any(|declared| declared.starts_with(prefix)),
        }
    }

    /// Check if a command is allowed for the given context.
    ///
    /// `command` is a canonical dispatch identifier: core commands use
    /// `"fs:read"`, native plugin commands use `"plugin:vault|read"`, and
    /// app commands may use a plain `"some-app-command"` identifier.
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
    /// **Default:** when no capabilities are loaded, returns `false`.
    /// Renderer access always requires an explicit startup grant.
    pub(crate) fn is_allowed(
        &self,
        command: &str,
        window: Option<&str>,
        webview: Option<&str>,
        origin: Option<&str>,
    ) -> bool {
        // Lock-free snapshot — single atomic load + Arc bump. Holds the
        // immutable state for the duration of the check; concurrent
        // mutators see a fresh snapshot on their next `state.load()`
        // without ever blocking this read.
        let snapshot = self.state.load();
        if snapshot.fail_closed {
            return false;
        }
        let resolved = match &snapshot.posture {
            Posture::DenyAll => return false,
            Posture::Configured(configured) => configured.resolved.as_ref(),
        };
        resolved.is_allowed(command, window, webview, origin)
    }
}

/// Shape validation plus `backend:<fn>` permission-reference checks.
fn validate_capability(capability: &Capability) -> Result<(), String> {
    validate_capability_shape(capability)?;
    for entry in &capability.permissions {
        if let PermissionEntry::PermissionRef(reference) = entry {
            if reference.strip_prefix(BACKEND_PREFIX) == Some("") {
                return Err(format!(
                    "capability '{}': permission '{reference}' must name a backend function \
                     (`backend:<fn>` or `backend:*`)",
                    capability.identifier
                ));
            }
        }
    }
    Ok(())
}

/// `backend:<fn>` permission references target user backend functions, which
/// have no plugin manifest to resolve against. Rewrite them into inline
/// permissions allowing the `backend:<fn>` command so they cannot fail the
/// resolution of the whole capability set.
fn normalize_backend_refs(capability: &Capability) -> Capability {
    let mut normalized = capability.clone();
    for entry in &mut normalized.permissions {
        let PermissionEntry::PermissionRef(reference) = entry else {
            continue;
        };
        if !reference.starts_with(BACKEND_PREFIX) {
            continue;
        }
        let command = std::mem::take(reference);
        *entry = PermissionEntry::Inline(Permission {
            version: None,
            identifier: command.clone(),
            description: None,
            commands: Commands {
                allow: vec![command],
                deny: Vec::new(),
            },
            scope: Value::Null,
            platforms: None,
        });
    }
    normalized
}

/// `api:` prefix of a command or permission reference, if it has one.
fn command_prefix(command: &str) -> Option<&str> {
    command.find(':').map(|index| &command[..=index])
}

fn declared_prefixes(capabilities: &BTreeMap<String, Capability>) -> BTreeSet<String> {
    let mut prefixes = BTreeSet::new();
    for capability in capabilities.values() {
        for entry in &capability.permissions {
            match entry {
                PermissionEntry::PermissionRef(reference) => {
                    prefixes.extend(command_prefix(reference).map(str::to_string));
                },
                PermissionEntry::Inline(permission) => {
                    let commands = permission
                        .commands
                        .allow
                        .iter()
                        .chain(&permission.commands.deny);
                    prefixes.extend(
                        commands
                            .filter_map(|c| command_prefix(c))
                            .map(str::to_string),
                    );
                },
            }
        }
    }
    prefixes
}

/// Process-wide authority, built on first use. Reached through [`authority`].
static AUTHORITY: LazyLock<Authority> = LazyLock::new(Authority::new);

/// Get the process-wide ACL authority.
pub(crate) fn authority() -> &'static Authority {
    &AUTHORITY
}

/// Generate a JSON-Schema describing the [`Capability`] format and
/// write it to `out_path`. Consumed by IDE autocomplete on
/// `*.tynd-capability.json` files.
///
/// Hand-rolled rather than relying on `schemars`; the shape mirrors
/// Tynd's internal [`Capability`] struct.
pub(crate) fn generate_schema(out_path: &Path) -> Result<(), String> {
    let schema = capability_schema();
    let serialized = serde_json::to_string_pretty(&schema).map_err(|e| {
        IpcError::new("acl", "schema", format!("serialize failed — {e}")).into_string()
    })?;
    std::fs::write(out_path, serialized)
        .map_err(|e| IpcError::new("acl", "schema", e).into_string())
}

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("acl", |method, args| dispatch(method, &args));

/// Dispatch `acl.*` IPC calls from the JS bridge. Mirrors the shape of
/// every other `api::*` dispatch fn — `Result<Value, String>` so the
/// frontend gets a textual error.
pub(crate) fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "isAllowed" => {
            let command = args.get("command").and_then(Value::as_str).ok_or_else(|| {
                String::from(IpcError::missing_arg("acl", "isAllowed", "command"))
            })?;
            let window = args.get("window").and_then(Value::as_str);
            let webview = args.get("webview").and_then(Value::as_str);
            let origin = args.get("origin").and_then(Value::as_str);
            Ok(Value::Bool(
                authority().is_allowed(command, window, webview, origin),
            ))
        },
        "list" => serde_json::to_value(authority().list()).map_err(|e| {
            IpcError::new("acl", "list", format!("serialize failed — {e}")).into_string()
        }),
        "generateSchema" => {
            let path = args.get("path").and_then(Value::as_str).ok_or_else(|| {
                String::from(IpcError::missing_arg("acl", "generateSchema", "path"))
            })?;
            let resolved = crate::security::policy::authorize_fs_path(path)
                .map_err(|e| IpcError::new("acl", "generateSchema", e).into_string())?;
            generate_schema(&resolved)?;
            Ok(Value::Null)
        },
        _ => Err(IpcError::unknown_method("acl", method).into_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn authority_with(value: Value) -> Authority {
        let authority = Authority::new();
        authority.load_value(value).expect("capabilities load");
        authority
    }

    fn cap(permissions: Value) -> Value {
        json!([{ "identifier": "main-cap", "windows": ["main"], "permissions": permissions }])
    }

    fn allow(commands: &[&str]) -> Value {
        json!({ "identifier": "p", "commands": { "allow": commands } })
    }

    #[test]
    fn backend_permission_ref_resolves_as_inline_allow() {
        let authority = authority_with(cap(json!(["backend:greet", allow(&["fs:read"])])));
        assert!(authority.is_allowed("backend:greet", Some("main"), Some("main"), None));
        assert!(!authority.is_allowed("backend:other", Some("main"), Some("main"), None));
        assert!(!authority.is_allowed("backend:greet", Some("other"), Some("other"), None));
        // The rest of the set still resolved.
        assert!(authority.is_allowed("fs:read", Some("main"), Some("main"), None));
        assert!(authority.declares_command_prefix("backend:"));
    }

    #[test]
    fn backend_wildcard_permission_ref_allows_all_functions() {
        let authority = authority_with(cap(json!(["backend:*"])));
        assert!(authority.is_allowed("backend:anything", Some("main"), Some("main"), None));
    }

    #[test]
    fn empty_backend_permission_ref_is_rejected_with_a_clear_error() {
        let authority = Authority::new();
        let error = authority
            .load_value(cap(json!(["backend:"])))
            .expect_err("empty backend ref");
        assert!(error.contains("must name a backend function"), "{error}");
    }

    #[test]
    fn declared_prefixes_are_cached_per_namespace() {
        let authority = authority_with(cap(json!([allow(&["fs:read", "window:setTitle"])])));
        assert!(authority.declares_command_prefix("fs:"));
        assert!(authority.declares_command_prefix("window:"));
        assert!(!authority.declares_command_prefix("backend:"));
        assert!(!authority.declares_command_prefix("tray:"));
        assert!(!Authority::new().declares_command_prefix("fs:"));
    }

    #[test]
    fn denied_commands_count_as_declared() {
        let authority = authority_with(cap(json!([
            { "identifier": "p", "commands": { "deny": ["backend:danger"] } }
        ])));
        assert!(authority.declares_command_prefix("backend:"));
    }

    #[test]
    fn failed_manifest_load_is_fail_closed_until_a_valid_load() {
        let authority = authority_with(cap(json!([allow(&["fs:read"])])));
        assert!(!authority.is_fail_closed());
        assert!(authority.is_allowed("fs:read", Some("main"), Some("main"), None));

        let error = authority.load_value(json!("not a capability file"));
        assert!(error.is_err());
        assert!(authority.is_fail_closed());
        assert!(!authority.is_allowed("fs:read", Some("main"), Some("main"), None));
        // No capability is left to declare anything: callers must use the flag.
        assert!(!authority.declares_command_prefix("backend:"));

        authority
            .load_value(cap(json!([allow(&["fs:read"])])))
            .expect("valid reload");
        assert!(!authority.is_fail_closed());
        assert!(authority.is_allowed("fs:read", Some("main"), Some("main"), None));
    }
}
