//! Plugin setup registry: init scripts and custom URI schemes that native
//! plugins register during `init`, read back by the webview builders when a
//! webview is created.

use std::sync::{Mutex, MutexGuard, PoisonError};

use tynd_plugin_sdk::UriHandler;

/// Maximum accepted size of one registered init script.
pub(crate) const MAX_INIT_SCRIPT_BYTES: usize = 256 * 1024;

/// Schemes the host owns or the webview treats specially; plugins cannot claim them.
const RESERVED_SCHEMES: &[&str] = &[
    "http",
    "https",
    "file",
    "data",
    "blob",
    "about",
    "javascript",
    "ws",
    "wss",
    "ftp",
    "mailto",
];

/// One registered init script and whether it may run in remote-content webviews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InitScriptEntry {
    pub(crate) script: String,
    /// `false` (default): installed only in webviews showing app-local content.
    pub(crate) remote: bool,
}

/// One registered URI scheme and whether remote-content pages may reach it.
#[derive(Clone)]
pub(crate) struct UriSchemeEntry {
    pub(crate) name: String,
    pub(crate) handler: UriHandler,
    /// `false` (default): installed only in app-local webviews and refused
    /// when the requesting page is remote.
    pub(crate) remote: bool,
}

impl std::fmt::Debug for UriSchemeEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UriSchemeEntry")
            .field("name", &self.name)
            .field("remote", &self.remote)
            .finish_non_exhaustive()
    }
}

static INIT_SCRIPTS: Mutex<Vec<InitScriptEntry>> = Mutex::new(Vec::new());
static URI_SCHEMES: Mutex<Vec<UriSchemeEntry>> = Mutex::new(Vec::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Queue a JavaScript snippet to run in every webview created afterwards,
/// before page scripts. `remote: false` restricts it to app-local webviews;
/// `remote: true` also runs it in webviews showing remote content (it must
/// then embed no secret). Oversized or blank scripts are rejected.
pub(crate) fn register_init_script(script: String, remote: bool) -> Result<(), String> {
    if script.trim().is_empty() {
        return Err("init script must not be empty".to_string());
    }
    if script.len() > MAX_INIT_SCRIPT_BYTES {
        return Err(format!(
            "init script exceeds {MAX_INIT_SCRIPT_BYTES} byte limit"
        ));
    }
    lock(&INIT_SCRIPTS).push(InitScriptEntry { script, remote });
    Ok(())
}

/// Snapshot of every registered init script with its `remote` flag. Callers
/// must filter on it: remote-origin webviews only get `remote: true` scripts.
pub(crate) fn init_script_entries() -> Vec<InitScriptEntry> {
    lock(&INIT_SCRIPTS).clone()
}

/// Whether `name` is a valid custom scheme (RFC 3986 `scheme`, lowercase) that
/// is not reserved by the host or the webview and is not a `tynd*` scheme.
pub(crate) fn validate_uri_scheme_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let valid = name.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '-' | '.'));
    if !valid {
        return Err(format!(
            "invalid URI scheme '{name}': use 1-32 chars of [a-z0-9+.-] starting with a letter"
        ));
    }
    if name.starts_with("tynd") || RESERVED_SCHEMES.contains(&name) {
        return Err(format!("URI scheme '{name}' is reserved"));
    }
    Ok(())
}

/// Register a custom URI scheme handler. `remote: false` keeps it reachable
/// from app-local webviews only; `remote: true` also serves pages that show
/// remote content (the handler must then treat every request as untrusted).
/// Fails on an invalid, reserved or already-registered name.
pub(crate) fn register_uri_scheme(
    name: impl Into<String>,
    handler: UriHandler,
    remote: bool,
) -> Result<(), String> {
    let name = name.into();
    validate_uri_scheme_name(&name)?;
    let mut schemes = lock(&URI_SCHEMES);
    if schemes.iter().any(|existing| existing.name == name) {
        return Err(format!("URI scheme '{name}' is already registered"));
    }
    schemes.push(UriSchemeEntry {
        name,
        handler,
        remote,
    });
    Ok(())
}

/// Snapshot of every registered scheme with its `remote` flag. Callers must
/// filter on it: remote-origin webviews only get `remote: true` schemes.
pub(crate) fn uri_scheme_entries() -> Vec<UriSchemeEntry> {
    lock(&URI_SCHEMES).clone()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tynd_plugin_sdk::{UriRequest, UriResponse};

    use super::*;

    // The registries are process-global; every test uses names/scripts unique
    // to it and asserts on membership, never on exact contents.

    #[test]
    fn init_scripts_register_in_order_and_reject_bad_input() {
        register_init_script("window.__hooks_a = 1;".to_string(), false).unwrap();
        register_init_script("window.__hooks_b = 2;".to_string(), false).unwrap();
        let scripts = init_script_entries();
        let a = scripts
            .iter()
            .position(|e| e.script == "window.__hooks_a = 1;");
        let b = scripts
            .iter()
            .position(|e| e.script == "window.__hooks_b = 2;");
        assert!(a.is_some() && b.is_some() && a < b);

        assert!(register_init_script("   ".to_string(), false).is_err());
        let huge = "x".repeat(MAX_INIT_SCRIPT_BYTES + 1);
        assert!(register_init_script(huge, false).is_err());
    }

    #[test]
    fn uri_scheme_registers_and_handler_is_callable() {
        let handler: UriHandler = Arc::new(|req| UriResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/plain".into())],
            body: req.url.into_bytes(),
        });
        register_uri_scheme("hookstest", handler, false).unwrap();
        let entry = uri_scheme_entries()
            .into_iter()
            .find(|entry| entry.name == "hookstest")
            .expect("scheme registered");
        let response = (entry.handler)(UriRequest {
            url: "hookstest://x/y".into(),
            ..UriRequest::default()
        });
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"hookstest://x/y");
    }

    #[test]
    fn registrations_are_local_only_unless_they_opt_in() {
        register_init_script("window.__hooks_local = 1;".to_string(), false).unwrap();
        register_init_script("window.__hooks_remote = 1;".to_string(), true).unwrap();
        let entries = init_script_entries();
        let remote_of = |script: &str| {
            entries
                .iter()
                .find(|entry| entry.script == script)
                .map(|entry| entry.remote)
        };
        assert_eq!(remote_of("window.__hooks_local = 1;"), Some(false));
        assert_eq!(remote_of("window.__hooks_remote = 1;"), Some(true));

        let handler: UriHandler = Arc::new(|_| UriResponse::default());
        register_uri_scheme("hookslocal", handler.clone(), false).unwrap();
        register_uri_scheme("hooksremote", handler.clone(), true).unwrap();
        assert!(register_uri_scheme("hooksremote", handler, false).is_err());
        let schemes = uri_scheme_entries();
        let remote_of = |name: &str| {
            schemes
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.remote)
        };
        assert_eq!(remote_of("hookslocal"), Some(false));
        assert_eq!(remote_of("hooksremote"), Some(true));
    }

    #[test]
    fn uri_scheme_rejects_duplicate_invalid_and_reserved() {
        let handler: UriHandler = Arc::new(|_| UriResponse::default());
        register_uri_scheme("hooksdup", handler.clone(), false).unwrap();
        assert!(register_uri_scheme("hooksdup", handler.clone(), false).is_err());
        for bad in [
            "",
            "Upper",
            "1abc",
            "has space",
            "tynd-evil",
            "tynd",
            "https",
            "file",
        ] {
            assert!(
                register_uri_scheme(bad, handler.clone(), false).is_err(),
                "{bad:?} should be rejected"
            );
        }
        assert!(register_uri_scheme("a".repeat(33), handler, false).is_err());
    }
}
