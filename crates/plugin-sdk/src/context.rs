use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::Value;

use crate::{
    current_caller, BackgroundHandle, CallerInfo, PluginError, PluginResult, PluginSetup,
    UriHandler,
};

/// Host-provided callback that emits a named event to every webview.
pub type EmitFn = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// Host-provided callback that routes a plugin's `cx.os_call` into the core OS dispatch table.
pub type OsDispatchFn = Arc<dyn Fn(&str, &str, Value) -> PluginResult<Value> + Send + Sync>;

/// Context handed to [`crate::TyndPlugin::dispatch`].
pub struct PluginCx {
    emit: EmitFn,
    os_dispatch: Option<OsDispatchFn>,
    allowed_permissions: Option<HashSet<String>>,
    setup: Option<Arc<dyn PluginSetup>>,
}

impl std::fmt::Debug for PluginCx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginCx").finish_non_exhaustive()
    }
}

fn normalize_api_key(api: &str) -> String {
    api.trim()
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_host_api(api: &str) -> String {
    match normalize_api_key(api).as_str() {
        "applescript" => "appleScript".to_string(),
        "deeplink" => "deepLink".to_string(),
        "networkloopback" => "network-loopback".to_string(),
        "networkpublic" => "network-public".to_string(),
        "singleinstance" => "singleInstance".to_string(),
        "systemstats" => "systemStats".to_string(),
        "windowstate" => "windowState".to_string(),
        other => other.to_string(),
    }
}

fn permission_key(api: &str, method: &str) -> String {
    format!("{}:{}", normalize_api_key(api), method.trim())
}

fn normalize_permission_selector(selector: &str) -> Option<String> {
    let selector = selector.trim();
    if selector.is_empty() {
        return None;
    }
    let (api, method) = selector.split_once(':').unwrap_or((selector, "*"));
    if api.trim().is_empty() || method.trim().is_empty() {
        return None;
    }
    Some(permission_key(api, method))
}

impl PluginCx {
    /// Build a context wired with the host's emit callback.
    pub fn new(emit: EmitFn) -> Self {
        Self {
            emit,
            os_dispatch: None,
            allowed_permissions: Some(HashSet::new()),
            setup: None,
        }
    }

    /// Build a noop context for unit tests.
    #[must_use]
    pub fn test() -> Self {
        Self::new(Arc::new(|_, _| {}))
    }

    /// Attach the host's OS dispatcher so `os_call` can route through it.
    #[must_use]
    pub fn with_dispatch(mut self, dispatch: OsDispatchFn) -> Self {
        self.os_dispatch = Some(dispatch);
        self
    }

    /// Attach the host's setup registry so plugins can register init scripts
    /// and custom URI schemes.
    #[must_use]
    pub fn with_setup(mut self, setup: Arc<dyn PluginSetup>) -> Self {
        self.setup = Some(setup);
        self
    }

    /// Restrict this context to command-level selectors.
    ///
    /// Selectors use the renderer capability shape (`"fs:readText"`).
    /// `"fs:*"` grants every method on one API. Bare `"fs"` is read as
    /// `"fs:*"`. Malformed selectors are
    /// ignored, preserving the deny-by-default boundary.
    #[must_use]
    pub fn with_allowed_permissions<I, S>(mut self, permissions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_permissions = Some(
            permissions
                .into_iter()
                .filter_map(|permission| normalize_permission_selector(permission.as_ref()))
                .collect(),
        );
        self
    }

    /// Clear any previously-set allow-list, switching to the unrestricted state.
    #[must_use]
    pub fn with_allowed_apis_unrestricted(mut self) -> Self {
        self.allowed_permissions = None;
        self
    }

    /// Renderer that issued the call currently being dispatched.
    ///
    /// `Some` only while [`crate::TyndPlugin::dispatch`] runs on the thread
    /// that received a renderer call; `None` for calls from the backend,
    /// `init`, `on_event`, `shutdown` and threads the plugin spawned itself.
    pub fn caller(&self) -> Option<CallerInfo> {
        current_caller()
    }

    /// Queue a JavaScript snippet to run in every webview that shows app-local
    /// content (bundled `tynd://` pages or a loopback dev server), created
    /// after this call, before page scripts. Remote-content webviews and
    /// remote frames never run it. Call it from [`crate::TyndPlugin::init`].
    ///
    /// A context without a host setup registry (unit tests) ignores the call.
    pub fn register_init_script(&self, script: impl Into<String>) -> PluginResult<()> {
        self.with_setup_sink(|setup| setup.init_script(script.into()))
    }

    /// Like [`Self::register_init_script`], but the script also runs in
    /// webviews showing remote content. It must embed no secret and assume
    /// hostile page scripts can observe everything it exposes.
    pub fn register_init_script_remote(&self, script: impl Into<String>) -> PluginResult<()> {
        self.with_setup_sink(|setup| setup.init_script_scoped(script.into(), true))
    }

    /// Register a custom URI scheme (`name://...`) served by `handler`. Call it
    /// from [`crate::TyndPlugin::init`]. Names must be lowercase `[a-z0-9+.-]`,
    /// start with a letter, and not collide with a reserved or already
    /// registered scheme.
    ///
    /// The scheme is installed only in webviews showing app-local content and
    /// requests whose page origin is remote are refused (403). Each
    /// [`UriRequest`](crate::UriRequest) carries the issuing `webview` and the
    /// page `origin`. Use [`Self::register_uri_scheme_remote`] to serve remote
    /// pages too.
    ///
    /// A context without a host setup registry (unit tests) ignores the call.
    pub fn register_uri_scheme(
        &self,
        name: impl Into<String>,
        handler: UriHandler,
    ) -> PluginResult<()> {
        self.with_setup_sink(|setup| setup.uri_scheme(name.into(), handler))
    }

    /// Like [`Self::register_uri_scheme`], but remote-content webviews can
    /// reach the scheme too. The handler must treat every request as untrusted
    /// and check [`UriRequest::origin`](crate::UriRequest::origin).
    pub fn register_uri_scheme_remote(
        &self,
        name: impl Into<String>,
        handler: UriHandler,
    ) -> PluginResult<()> {
        self.with_setup_sink(|setup| setup.uri_scheme_scoped(name.into(), handler, true))
    }

    /// Run `register` against the host setup sink; a context without one
    /// (unit tests) skips it.
    fn with_setup_sink(
        &self,
        register: impl FnOnce(&dyn PluginSetup) -> Result<(), String>,
    ) -> PluginResult<()> {
        match &self.setup {
            Some(setup) => register(setup.as_ref()).map_err(PluginError::Custom),
            None => Ok(()),
        }
    }

    /// Emit an event to all webviews. Fire-and-forget.
    pub fn emit(&self, event: &str, payload: Value) {
        (self.emit)(event, payload);
    }

    /// Clone-able handle on the emit callback.
    pub fn emitter(&self) -> EmitFn {
        Arc::clone(&self.emit)
    }

    /// Invoke a core OS API (`fs`, `http`, `dialog`, ...) from a plugin.
    pub fn os_call(&self, api: &str, method: &str, args: Value) -> PluginResult<Value> {
        let norm_api = normalize_api_key(api);
        if let Some(allowed) = &self.allowed_permissions {
            let exact = permission_key(api, method);
            let wildcard = format!("{norm_api}:*");
            if !allowed.contains(&exact) && !allowed.contains(&wildcard) {
                let mut allowed_vec: Vec<String> = allowed.iter().cloned().collect();
                allowed_vec.sort();
                return Err(PluginError::NotPermitted {
                    api: api.to_string(),
                    method: method.to_string(),
                    allowed: allowed_vec,
                });
            }
        }
        let dispatch = self
            .os_dispatch
            .as_ref()
            .ok_or(PluginError::DispatchNotWired)?;
        let host_api = canonical_host_api(api);
        dispatch(&host_api, method, args)
    }

    /// Authorize a filesystem path and return the host-resolved path that
    /// ambient filesystem APIs must consume.
    ///
    /// The authorization is a point-in-time check. Consumers that require a
    /// race-free boundary must use a host capability API instead of opening
    /// this path later through the ambient filesystem.
    pub fn authorize_fs_path(&self, path: &str, operation: &str) -> PluginResult<String> {
        let resolved = self.os_call(
            "security",
            "checkFs",
            serde_json::json!({ "path": path, "op": operation }),
        )?;
        resolved.as_str().map(str::to_owned).ok_or_else(|| {
            PluginError::Custom(
                "security.checkFs: host returned an invalid resolved path".to_string(),
            )
        })
    }

    /// Spawn a background thread firing `body` every `interval`.
    ///
    /// Returns an error when the operating system cannot create the worker
    /// thread. Dropping the returned handle stops and joins the worker.
    pub fn spawn_interval<F>(
        &self,
        name: impl Into<String>,
        interval: Duration,
        mut body: F,
    ) -> PluginResult<BackgroundHandle>
    where
        F: FnMut() + Send + 'static,
    {
        let running = Arc::new(AtomicBool::new(true));
        let r = Arc::clone(&running);
        let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        let name = name.into();
        let join = thread::Builder::new().name(name).spawn(move || {
            while r.load(Ordering::Acquire) {
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(&mut body));
                if let Err(payload) = res {
                    let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                        (*s).to_string()
                    } else if let Some(s) = payload.downcast_ref::<String>() {
                        s.clone()
                    } else {
                        "<non-string panic>".to_string()
                    };
                    tracing::error!(
                        target: "tynd::plugin_sdk",
                        "spawn_interval body panicked: {msg}; continuing on next tick"
                    );
                }
                match stop_rx.recv_timeout(interval) {
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    _ => break,
                }
            }
        })?;
        Ok(BackgroundHandle::new(running, stop_tx, Some(join)))
    }
}

/// Event recorder returned by [`test_cx`].
#[derive(Clone, Debug, Default)]
pub struct EventCapture {
    inner: Arc<Mutex<Vec<(String, Value)>>>,
}

impl EventCapture {
    /// Snapshot every event captured so far, in emission order.
    pub fn events(&self) -> Vec<(String, Value)> {
        self.inner.lock().clone()
    }

    /// Number of events captured so far.
    pub fn len(&self) -> usize {
        self.inner.lock().len()
    }

    /// Whether anything has been emitted yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Reset the recorder.
    pub fn clear(&self) {
        self.inner.lock().clear();
    }
}

/// Build a [`PluginCx`] pre-wired for unit tests.
#[must_use]
pub fn test_cx() -> (PluginCx, EventCapture) {
    let capture = EventCapture::default();
    let inner = Arc::clone(&capture.inner);
    let emit: EmitFn = Arc::new(move |name: &str, payload: Value| {
        inner.lock().push((name.to_string(), payload));
    });
    (PluginCx::new(emit), capture)
}

/// Strip non-ASCII + control bytes and cap to 256 bytes for event-safe strings.
#[must_use]
pub fn sanitize_event_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(256));
    for ch in s.chars() {
        if out.len() >= 256 {
            break;
        }
        if !ch.is_ascii() || ch.is_ascii_control() {
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{with_caller, UriRequest, UriResponse};

    #[derive(Default)]
    struct FakeSetup {
        scripts: Mutex<Vec<String>>,
        schemes: Mutex<Vec<String>>,
    }

    impl PluginSetup for FakeSetup {
        fn init_script(&self, script: String) -> Result<(), String> {
            self.scripts.lock().push(script);
            Ok(())
        }

        fn uri_scheme(&self, name: String, _handler: UriHandler) -> Result<(), String> {
            if name == "taken" {
                return Err("taken".to_string());
            }
            self.schemes.lock().push(name);
            Ok(())
        }
    }

    #[test]
    fn caller_is_visible_only_inside_with_caller() {
        let (cx, _) = test_cx();
        assert_eq!(cx.caller(), None);
        let info = CallerInfo {
            window: "main".into(),
            webview: "main".into(),
            origin: Some("https://example.com".into()),
        };
        with_caller(Some(info.clone()), || {
            assert_eq!(cx.caller(), Some(info));
        });
        assert_eq!(cx.caller(), None);
    }

    #[test]
    fn setup_registrations_reach_the_sink_and_errors_propagate() {
        let sink = Arc::new(FakeSetup::default());
        let (cx, _) = test_cx();
        let cx = cx.with_setup(sink.clone());
        let handler: UriHandler = Arc::new(|_| UriResponse::default());

        cx.register_init_script("window.x = 1;").unwrap();
        cx.register_uri_scheme("mine", handler.clone()).unwrap();
        let err = cx.register_uri_scheme("taken", handler).unwrap_err();

        assert_eq!(*sink.scripts.lock(), vec!["window.x = 1;".to_string()]);
        assert_eq!(*sink.schemes.lock(), vec!["mine".to_string()]);
        assert!(err.to_string().contains("taken"));
    }

    #[test]
    fn setup_calls_without_a_sink_are_ignored() {
        let (cx, _) = test_cx();
        cx.register_init_script("x").unwrap();
        cx.register_uri_scheme("x", Arc::new(|_| UriResponse::default()))
            .unwrap();
    }

    /// Sink that records the `remote` flag of the scoped registrations.
    #[derive(Debug, Default)]
    struct ScopedSetup {
        calls: Mutex<Vec<(String, bool)>>,
    }

    impl PluginSetup for ScopedSetup {
        fn init_script(&self, script: String) -> Result<(), String> {
            self.init_script_scoped(script, false)
        }

        fn uri_scheme(&self, name: String, handler: UriHandler) -> Result<(), String> {
            self.uri_scheme_scoped(name, handler, false)
        }

        fn init_script_scoped(&self, script: String, remote: bool) -> Result<(), String> {
            self.calls.lock().push((script, remote));
            Ok(())
        }

        fn uri_scheme_scoped(
            &self,
            name: String,
            _handler: UriHandler,
            remote: bool,
        ) -> Result<(), String> {
            self.calls.lock().push((name, remote));
            Ok(())
        }
    }

    #[test]
    fn registrations_are_local_unless_the_remote_variant_is_used() {
        let sink = Arc::new(ScopedSetup::default());
        let (cx, _) = test_cx();
        let cx = cx.with_setup(sink.clone());
        let handler: UriHandler = Arc::new(|_| UriResponse::default());

        cx.register_init_script("local-script").unwrap();
        cx.register_init_script_remote("remote-script").unwrap();
        cx.register_uri_scheme("localscheme", handler.clone())
            .unwrap();
        cx.register_uri_scheme_remote("remotescheme", handler)
            .unwrap();

        assert_eq!(
            *sink.calls.lock(),
            vec![
                ("local-script".to_string(), false),
                ("remote-script".to_string(), true),
                ("localscheme".to_string(), false),
                ("remotescheme".to_string(), true),
            ]
        );
    }

    #[test]
    fn sinks_that_predate_the_remote_flag_stay_local_only() {
        let sink = Arc::new(FakeSetup::default());
        let (cx, _) = test_cx();
        let cx = cx.with_setup(sink.clone());
        cx.register_init_script_remote("legacy").unwrap();
        cx.register_uri_scheme_remote("legacy", Arc::new(|_| UriResponse::default()))
            .unwrap();
        // The default `*_scoped` methods fall back to the local registration.
        assert_eq!(*sink.scripts.lock(), vec!["legacy".to_string()]);
        assert_eq!(*sink.schemes.lock(), vec!["legacy".to_string()]);
    }

    #[test]
    fn uri_request_carries_webview_and_origin() {
        let request = UriRequest {
            webview: "panel".into(),
            origin: Some("https://example.com".into()),
            ..UriRequest::default()
        };
        assert_eq!(request.webview, "panel");
        assert_eq!(request.origin.as_deref(), Some("https://example.com"));
        assert_eq!(UriRequest::default().origin, None);
    }
}
