//! Routes every message coming in over the WebView IPC channel.
//!
//! The WebView handler runs on a wry-owned thread — we keep that closure
//! minimal and delegate the parsing + fan-out here.

use std::{fmt, sync::LazyLock};

use compact_str::CompactString;
use crossbeam_channel as mpsc;
use dashmap::DashMap;
use serde::{
    de::{self, MapAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::Value;
use tao::event_loop::EventLoopProxy;
use tracing::warn;

use crate::{api, app::UserEvent};
use tynd_protocol::{
    runtime::{BackendCall, BackendEvent, BackendReturnPayload},
    MAX_IPC_FRAME_BYTES,
};

/// Maps backend call `id` to the window `label` that issued it. Needed so the
/// main loop can route `BackendEvent::Yield` / `Return` back to the webview
/// that originated the streaming call (otherwise secondary windows never see
/// their own yields). Entries are cleared on `Return`. `LazyLock` builds
/// the `DashMap` on first deref so callers reach it via direct
/// `CALL_LABELS.get(...)` instead of a `call_labels()` accessor.
///
/// Value type is `CompactString`: window labels are typically short
/// (`"primary"`, `"main"`, user-supplied window names ≤24 B) and inline
/// without a heap alloc. Keys stay `String` because UUIDs (36 chars) spill
/// past CompactString's 24-byte inline cap on 64-bit anyway.
/// Move a string field out of a `Value` without cloning. Falls back to an
/// empty string if the key is missing or not a string.
fn take_string(v: &mut Value, key: &str) -> String {
    match v.get_mut(key).map(std::mem::take) {
        Some(Value::String(s)) => s,
        _ => String::new(),
    }
}

pub(crate) static CALL_LABELS: LazyLock<DashMap<String, CompactString>> =
    LazyLock::new(DashMap::default);

/// Cancel every backend call issued by the webview `label`, streaming or not:
/// its window closed or its document went away (navigation, reload), so no
/// page is left to receive the answer. The backend aborts the handler's
/// `getCallContext().signal` and drops its result.
pub(crate) fn cancel_calls_for_label(label: &str, call_tx: &mpsc::Sender<BackendCall>) {
    // Inline-4 covers the typical 0-2 in-flight calls per webview without a
    // heap alloc; spills transparently for higher fan-out.
    let orphaned: smallvec::SmallVec<[String; 4]> = CALL_LABELS
        .iter()
        .filter_map(|entry| (entry.value() == label).then(|| entry.key().clone()))
        .collect();
    for id in orphaned {
        CALL_LABELS.remove(&id);
        let _ = call_tx.send(BackendCall::Cancel { id });
    }
}

/// Best-effort ID recovery for an oversized frame that cannot safely be
/// parsed. Never use this raw scan for authorization, routing an accepted
/// call, or backend dispatch.
fn extract_call_id(body: &str) -> Option<&str> {
    let after = body.split_once(r#""id":""#)?.1;
    let end = after.find('"')?;
    Some(&after[..end])
}

/// Prefix of the framework-reserved call ids / names (`__tynd_on_close__`,
/// `__tynd_fe_emit__`, ...). The host and the runtime give these meaning, so a
/// renderer must never choose one as a call id: the backend echoes the id in
/// its `Return`, and the host treats a `Return` with a reserved id as an
/// internal signal (e.g. "exit now").
const RESERVED_PREFIX: &str = "__tynd_";

fn valid_call_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.starts_with(RESERVED_PREFIX)
}

fn plugin_command_from_wire(fn_name: &str) -> Option<String> {
    let rest = fn_name.strip_prefix("$plugin$")?;
    let (plugin, method) = rest.split_once('$')?;
    if plugin.is_empty() || method.is_empty() || method.contains('$') {
        return None;
    }
    tynd_capabilities::plugin_command_key(plugin, method).ok()
}

pub(crate) fn handle_ipc_body_with_context(
    body: String,
    label: &str,
    window_label: &str,
    webview_label: &str,
    origin: Option<String>,
    call_tx: &mpsc::Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    let context = IpcContext {
        label,
        window_label,
        webview_label,
        origin: origin.as_deref(),
        call_tx,
        proxy,
    };
    if body.trim().is_empty() || reject_oversized_body(&body, &context) {
        return;
    }
    let Some(value) = parse_ipc_body(&body) else {
        return;
    };
    if value.get("type").and_then(Value::as_str) == Some("call") {
        handle_backend_call(value, &context);
        return;
    }
    handle_parsed_message(value, &context);
}

struct IpcContext<'a> {
    label: &'a str,
    window_label: &'a str,
    webview_label: &'a str,
    origin: Option<&'a str>,
    call_tx: &'a mpsc::Sender<BackendCall>,
    proxy: &'a EventLoopProxy<UserEvent>,
}

fn reject_oversized_body(body: &str, context: &IpcContext<'_>) -> bool {
    if body.len() <= MAX_IPC_FRAME_BYTES {
        return false;
    }
    warn!(
        "ipc body {} bytes exceeds {} cap — dropping",
        body.len(),
        MAX_IPC_FRAME_BYTES
    );
    if let Some(id) = extract_call_id(body)
        .filter(|id| valid_call_id(id))
        .map(str::to_string)
    {
        let message = format!(
            "IPC frame exceeds {MAX_IPC_FRAME_BYTES} byte limit; use binary APIs for large payloads"
        );
        // The frame is not parsed, so its kind is unknown: settle both the
        // backend-call and the OS-call promise. The shim ignores an id it does
        // not track.
        reject_call(&id, &message, context);
        let _ = context.proxy.send_event(UserEvent::OsResult {
            label: context.label.into(),
            id,
            ok: false,
            value: Value::String(message),
        });
    }
    true
}

/// Server-side caller identity forwarded to the backend as `ctx`.
///
/// The webview's own `ctx` key is always overwritten: the frontend must never
/// be able to choose the identity a backend handler sees.
fn inject_call_context(envelope: &mut Value, window: &str, webview: &str, origin: Option<&str>) {
    if let Some(map) = envelope.as_object_mut() {
        map.insert(
            "ctx".to_string(),
            serde_json::json!({ "window": window, "webview": webview, "origin": origin }),
        );
    }
}

/// ACL command name of a user backend function.
fn backend_command(fn_name: &str) -> String {
    format!("backend:{fn_name}")
}

/// Backend calls from the bundled app are only ACL-checked once the app opts
/// in by declaring at least one `backend:` command; until then every backend
/// function is allowed for the bundled app (opt-in ACL). A remote origin
/// never gets that shortcut: it needs an explicit grant even when no
/// `backend:` command is declared. A fail-closed authority (a capability
/// manifest failed to load) denies every call instead.
fn backend_access_allowed(
    fail_closed: bool,
    declares_backend: bool,
    remote_origin: bool,
    is_allowed: impl FnOnce() -> bool,
) -> bool {
    !fail_closed && ((!declares_backend && !remote_origin) || is_allowed())
}

/// The one framework-internal RPC name (frontend `emit()`). It is not a user
/// backend function: it is not gated by per-function `backend:` permissions,
/// otherwise declaring one `backend:` command silently breaks `emit()`. The
/// runtime refuses to dispatch every other `__tynd_*__` name to user exports.
const FRONTEND_EMIT_FN: &str = "__tynd_fe_emit__";

/// A framework-reserved name the renderer may not call (everything under the
/// `__tynd_` prefix except the frontend `emit()` RPC).
fn is_forbidden_reserved_fn(fn_name: &str) -> bool {
    fn_name.starts_with(RESERVED_PREFIX) && fn_name != FRONTEND_EMIT_FN
}

fn is_reserved_backend_fn(fn_name: &str) -> bool {
    fn_name == FRONTEND_EMIT_FN
}

/// Reserved calls skip the per-function `backend:` list but still honour a
/// fail-closed authority, and a remote origin still needs an explicit grant.
fn reserved_access_allowed(
    fail_closed: bool,
    remote_origin: bool,
    is_allowed: impl FnOnce() -> bool,
) -> bool {
    !fail_closed && (!remote_origin || is_allowed())
}

/// Returns the denied `backend:<fn>` command when the policy rejects this call.
fn denied_backend_command(fn_name: &str, context: &IpcContext<'_>) -> Option<String> {
    let authority = crate::security::acl::authority();
    let command = backend_command(fn_name);
    if is_reserved_backend_fn(fn_name) {
        let allowed =
            reserved_access_allowed(authority.is_fail_closed(), context.origin.is_some(), || {
                authority.is_allowed(
                    &command,
                    Some(context.window_label),
                    Some(context.webview_label),
                    context.origin,
                )
            });
        return (!allowed).then_some(command);
    }
    let allowed = backend_access_allowed(
        authority.is_fail_closed(),
        authority.declares_command_prefix("backend:"),
        context.origin.is_some(),
        || {
            authority.is_allowed(
                &command,
                Some(context.window_label),
                Some(context.webview_label),
                context.origin,
            )
        },
    );
    (!allowed).then_some(command)
}

/// Window commands that expose HttpOnly cookies / session tokens or run code in
/// a webview. These always need an explicit grant, exactly like the 29 gated
/// OS APIs.
const SENSITIVE_WINDOW_COMMANDS: &[&str] = &[
    "getWebviewCookies",
    "setWebviewCookie",
    "deleteWebviewCookie",
    "clearWebviewCookies",
    // Runs code in, or wipes the storage of, a webview (possibly another one).
    "evaluateScript",
    "clearBrowsingData",
];

/// Whether `<api>:<method>` needs an explicit grant. Window, tray and menu
/// commands are routed through the event loop and default apps use them freely
/// (no capability grants them by default), so calls from the bundled app are
/// only checked once the capabilities declare something in that namespace
/// (`window:*`, `tray:*`, `menu:*`), like `backend:` calls. Cookie commands
/// and every call from a remote origin always require an explicit grant.
fn native_access_allowed(
    api: &str,
    method: &str,
    fail_closed: bool,
    declares_namespace: bool,
    remote_origin: bool,
    is_allowed: impl FnOnce() -> bool,
) -> bool {
    if fail_closed {
        return false;
    }
    let sensitive = api == "window" && SENSITIVE_WINDOW_COMMANDS.contains(&method);
    (!declares_namespace && !sensitive && !remote_origin) || is_allowed()
}

/// A form of an otherwise default-open command that always needs an explicit
/// grant of that command:
/// - `window:printPage` with `silent: true` spools a job to a printer without
///   the user ever seeing a dialog;
/// - any webview session command (`capturePage`, `printToPdf`...) whose
///   `label` targets another webview than the caller (`caller_webview`): it
///   reads or drives that webview's content, which may be another origin.
fn is_sensitive_call_form(api: &str, method: &str, args: &Value, caller_webview: &str) -> bool {
    if api != "window" {
        return false;
    }
    if method == "printPage" && args.get("silent").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    crate::webview::session::is_session_method(method)
        && args
            .get("label")
            .and_then(Value::as_str)
            .is_some_and(|label| label != caller_webview)
}

/// [`native_access_allowed`] plus the argument-dependent sensitive forms
/// (`sensitive_form`, from [`is_sensitive_call_form`]). Each flag is a
/// separate policy fact, kept as plain arguments so tests read as a table.
#[allow(clippy::fn_params_excessive_bools)]
fn native_call_allowed(
    api: &str,
    method: &str,
    sensitive_form: bool,
    fail_closed: bool,
    declares_namespace: bool,
    remote_origin: bool,
    is_allowed: impl FnOnce() -> bool,
) -> bool {
    if sensitive_form {
        return !fail_closed && is_allowed();
    }
    native_access_allowed(
        api,
        method,
        fail_closed,
        declares_namespace,
        remote_origin,
        is_allowed,
    )
}

/// Returns the denied `<api>:<method>` command for a renderer `window.*`,
/// `tray.*` or `menu.*` call.
fn denied_native_command(
    api: &str,
    method: &str,
    args: &Value,
    context: &IpcContext<'_>,
) -> Option<String> {
    let authority = crate::security::acl::authority();
    let command = format!("{api}:{method}");
    let allowed = native_call_allowed(
        api,
        method,
        is_sensitive_call_form(api, method, args, context.webview_label),
        authority.is_fail_closed(),
        authority.declares_command_prefix(&format!("{api}:")),
        context.origin.is_some(),
        || {
            authority.is_allowed(
                &command,
                Some(context.window_label),
                Some(context.webview_label),
                context.origin,
            )
        },
    );
    (!allowed).then_some(command)
}

/// Command label for a `$plugin$` call that does not parse; the call is denied.
const INVALID_PLUGIN_COMMAND: &str = "plugin:<invalid>";

/// Plugin calls (`$plugin$<name>$<method>`) are checked against the plugin
/// ACL. A name that has the plugin prefix but does not parse is denied rather
/// than falling through to the (default-open) `backend:` rule, so validator
/// drift between the JS and Rust sides can never bypass the plugin ACL.
fn denied_wire_command(fn_name: &str, context: &IpcContext<'_>) -> Option<String> {
    if !fn_name.starts_with("$plugin$") {
        return denied_backend_command(fn_name, context);
    }
    let Some(command) = (fn_name.len() <= 256)
        .then(|| plugin_command_from_wire(fn_name))
        .flatten()
    else {
        return Some(INVALID_PLUGIN_COMMAND.to_string());
    };
    (!crate::security::acl::authority().is_allowed(
        &command,
        Some(context.window_label),
        Some(context.webview_label),
        context.origin,
    ))
    .then_some(command)
}

fn handle_backend_call(mut envelope: Value, context: &IpcContext<'_>) {
    if envelope
        .get("fn")
        .and_then(Value::as_str)
        .is_some_and(is_forbidden_reserved_fn)
    {
        warn!("ipc call: reserved framework function name rejected");
        if let Some(id) = envelope
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| valid_call_id(id))
        {
            reject_call(id, "reserved function name", context);
        }
        return;
    }

    let denied = envelope
        .get("fn")
        .and_then(Value::as_str)
        .and_then(|name| denied_wire_command(name, context));
    if let Some(command) = denied {
        reject_denied_call(
            envelope.get("id").and_then(Value::as_str),
            &command,
            context,
        );
        return;
    }

    inject_call_context(
        &mut envelope,
        context.window_label,
        context.webview_label,
        context.origin,
    );

    let Some(id_str) = envelope.get("id").and_then(Value::as_str) else {
        let Some(body) = serialize_ipc_envelope(&envelope) else {
            return;
        };
        if let Err(error) = context.call_tx.send(BackendCall::Raw(body)) {
            warn!("ipc call channel send failed (no id): {error}");
        }
        return;
    };
    if !valid_call_id(id_str) {
        warn!("ipc call: id must contain 1..=128 bytes");
        crate::ipc::events::emit(
            "ipc:error",
            &Value::String("call id must contain 1..=128 bytes".to_string()),
        );
        return;
    }

    let id = id_str.to_string();
    let Some(body) = serialize_ipc_envelope(&envelope) else {
        return;
    };
    record_ipc_request(envelope, &id, body.len());
    CALL_LABELS.insert(id.clone(), CompactString::from(context.label));
    if let Err(error) = context.call_tx.send(BackendCall::Raw(body)) {
        warn!("ipc call channel send failed: {error}");
        CALL_LABELS.remove(&id);
        reject_call(&id, "backend channel closed", context);
    }
}

/// Settle a renderer backend call with an error. Backend calls live in the
/// shim's `_pending` map, which only a backend `Return` (`__tynd_resolve__`)
/// settles; `OsResult` settles the separate OS-call map and would leave the
/// promise pending forever.
fn reject_call(id: &str, message: &str, context: &IpcContext<'_>) {
    CALL_LABELS.insert(id.to_string(), CompactString::from(context.label));
    let _ = context.proxy.send_event(call_rejection_event(id, message));
}

fn call_rejection_event(id: &str, message: &str) -> UserEvent {
    UserEvent::Backend(BackendEvent::Return {
        id: id.to_string(),
        payload: BackendReturnPayload::Err(message.to_string()),
    })
}

fn reject_denied_call(id: Option<&str>, command: &str, context: &IpcContext<'_>) {
    let Some(id) = id.filter(|id| valid_call_id(id)) else {
        warn!("denied plugin call '{command}' had no id");
        return;
    };
    reject_call(
        id,
        &format!("renderer command '{command}' denied by capability policy"),
        context,
    );
}

/// Rejects a denied `window.*` / `tray.*` / `menu.*` call through the regular
/// OS result path so the renderer promise settles with the denial.
fn reject_denied_os_call(id: String, command: &str, context: &IpcContext<'_>) {
    send_user_event(
        context,
        UserEvent::OsResult {
            label: context.label.into(),
            id,
            ok: false,
            value: Value::String(format!(
                "renderer command '{command}' denied by capability policy"
            )),
        },
        "OsResult",
    );
}

fn record_ipc_request(envelope: Value, id: &str, encoded_len: usize) {
    #[cfg(feature = "debug-protocol")]
    if crate::debug::ipc_tap_enabled() {
        let method = envelope
            .get("fn")
            .and_then(Value::as_str)
            .map(str::to_string);
        let captured = if encoded_len <= crate::debug::IPC_CAPTURE_MAX_BYTES {
            envelope
        } else {
            serde_json::json!({
                "__tynd_capture_truncated": true,
                "bytes": encoded_len,
            })
        };
        crate::debug::record_ipc_request(id, method.as_deref(), captured);
    }
    #[cfg(not(feature = "debug-protocol"))]
    let _ = (envelope, id, encoded_len);
}

fn parse_ipc_body(body: &str) -> Option<Value> {
    match serde_json::from_str(body) {
        Ok(UniqueIpcEnvelope(value)) => Some(value),
        Err(error) => {
            let message = format!("invalid ipc body: {error}");
            warn!("{message}");
            crate::ipc::events::emit("ipc:error", &Value::String(message));
            None
        },
    }
}

fn serialize_ipc_envelope(envelope: &Value) -> Option<String> {
    match serde_json::to_string(envelope) {
        Ok(body) => Some(body),
        Err(error) => {
            warn!("ipc call envelope serialization failed: {error}");
            crate::ipc::events::emit(
                "ipc:error",
                &Value::String("ipc call envelope serialization failed".to_string()),
            );
            None
        },
    }
}

/// Parsed IPC object whose decoded top-level keys are unique.
///
/// Authorization and backend dispatch must consume the same decoded
/// `type`/`id`/`fn` values. Serde's default `Value` deserializer keeps the last
/// duplicate object key, which would let an attacker present one `fn` to the
/// ACL and another to the JavaScript runtime.
struct UniqueIpcEnvelope(Value);

impl<'de> Deserialize<'de> for UniqueIpcEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(UniqueIpcEnvelopeVisitor)
    }
}

struct UniqueIpcEnvelopeVisitor;

impl<'de> Visitor<'de> for UniqueIpcEnvelopeVisitor {
    type Value = UniqueIpcEnvelope;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON IPC envelope object with unique keys")
    }

    fn visit_map<M>(self, mut entries: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut envelope = serde_json::Map::new();
        while let Some(key) = entries.next_key::<String>()? {
            if envelope.contains_key(&key) {
                return Err(de::Error::custom("duplicate key in IPC envelope"));
            }
            envelope.insert(key, entries.next_value()?);
        }
        Ok(UniqueIpcEnvelope(Value::Object(envelope)))
    }
}

fn handle_parsed_message(mut value: Value, context: &IpcContext<'_>) {
    let message_type = value
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    match message_type.as_deref() {
        Some("cancel") => handle_cancel(&mut value, context.call_tx),
        Some("ack") => handle_ack(&mut value, context.call_tx),
        Some("os_call") => handle_os_call(value, context),
        _ if is_page_ready(&value) => {
            if let Err(error) = context.proxy.send_event(UserEvent::PageReady {
                label: context.label.to_string(),
            }) {
                warn!("PageReady proxy.send_event failed: {error}");
            }
        },
        _ => {},
    }
}

fn handle_cancel(value: &mut Value, call_tx: &mpsc::Sender<BackendCall>) {
    let id = take_string(value, "id");
    if !valid_call_id(&id) {
        warn!("ipc cancel: missing, empty or reserved id — stream not cancelled");
        crate::ipc::events::emit(
            "ipc:error",
            &Value::String("cancel frame missing id".to_string()),
        );
        return;
    }
    CALL_LABELS.remove(&id);
    if let Err(error) = call_tx.send(BackendCall::Cancel { id }) {
        warn!("ipc cancel channel send failed: {error}");
    }
}

fn handle_ack(value: &mut Value, call_tx: &mpsc::Sender<BackendCall>) {
    let id = take_string(value, "id");
    let count = value
        .get("n")
        .and_then(Value::as_u64)
        .map_or(0, |count| count.min(u64::from(u32::MAX)) as u32);
    if valid_call_id(&id) && count > 0 {
        if let Err(error) = call_tx.send(BackendCall::Ack { id, n: count }) {
            warn!("ipc ack channel send failed: {error}");
        }
    }
}

fn is_page_ready(value: &Value) -> bool {
    value
        .get("__tynd_page_ready")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn handle_os_call(mut value: Value, context: &IpcContext<'_>) {
    let id = take_string(&mut value, "id");
    let api = take_string(&mut value, "api");
    let method = take_string(&mut value, "method");
    let args = value.get_mut("args").map_or(Value::Null, std::mem::take);

    if matches!(api.as_str(), "window" | "tray" | "menu") {
        if let Some(command) = denied_native_command(&api, &method, &args, context) {
            reject_denied_os_call(id, &command, context);
            return;
        }
    }

    match api.as_str() {
        "window" => send_user_event(
            context,
            UserEvent::WindowCmd {
                label: context.label.into(),
                id,
                method,
                args,
                origin: context.origin.map(str::to_owned),
            },
            "WindowCmd",
        ),
        "tray" => send_user_event(
            context,
            UserEvent::TrayCmd {
                label: context.label.into(),
                id,
                method,
                args,
                origin: context.origin.map(str::to_owned),
            },
            "TrayCmd",
        ),
        "menu" => send_user_event(
            context,
            UserEvent::MenuCmd {
                label: context.label.into(),
                id,
                method,
                args,
            },
            "MenuCmd",
        ),
        _ => submit_os_call(api, method, args, id, context),
    }
}

fn send_user_event(context: &IpcContext<'_>, event: UserEvent, name: &str) {
    if let Err(error) = context.proxy.send_event(event) {
        warn!("{name} proxy.send_event failed: {error}");
    }
}
/// One renderer OS call, owned so it can move onto the call pool.
struct OsCallJob {
    api: String,
    method: String,
    args: Value,
    window_label: String,
    webview_label: String,
    origin: Option<String>,
    generation: u64,
}

impl OsCallJob {
    /// Dispatch the call and return `(ok, value)`. A panic becomes an error
    /// result instead of unwinding into the call pool.
    fn run(self, id: &str) -> (bool, Value) {
        let Self {
            api,
            method,
            args,
            window_label,
            webview_label,
            origin,
            generation,
        } = self;
        let dispatched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let call = || {
                api::dispatch_webview_context(
                    &api,
                    &method,
                    args,
                    &window_label,
                    &webview_label,
                    origin.as_deref(),
                    generation,
                )
            };
            // Native plugins read the renderer identity through
            // `PluginCx::caller()` during this synchronous dispatch.
            #[cfg(feature = "plugins")]
            {
                crate::plugins::with_call_context(
                    &window_label,
                    &webview_label,
                    origin.as_deref(),
                    call,
                )
            }
            #[cfg(not(feature = "plugins"))]
            {
                call()
            }
        }));
        match dispatched {
            Ok(Ok(value)) => (true, value),
            Ok(Err(error)) => (false, Value::String(error)),
            Err(panic) => {
                let message = panic_message(panic.as_ref());
                tracing::error!(
                    target: "tynd::os::dispatch",
                    api = %api,
                    method = %method,
                    id = %id,
                    "os_call panicked: {message}",
                );
                (false, Value::String(format!("os_call panicked: {message}")))
            },
        }
    }
}

fn submit_os_call(api: String, method: String, args: Value, id: String, context: &IpcContext<'_>) {
    let job = OsCallJob {
        api,
        method,
        args,
        window_label: context.window_label.to_string(),
        webview_label: context.webview_label.to_string(),
        origin: context.origin.map(str::to_string),
        // Read on the IPC thread: a call received from the current document
        // keeps that generation even if it only runs after the next commit.
        generation: crate::ipc::owner::generation(context.webview_label),
    };
    let proxy = context.proxy.clone();
    let label = context.label.to_string();
    let fallback_proxy = proxy.clone();
    let fallback_label = label.clone();
    let fallback_id = id.clone();
    if let Err(error) = crate::ipc::call_pool::submit(move || {
        let (ok, value) = job.run(&id);
        if let Err(error) = proxy.send_event(UserEvent::OsResult {
            label,
            id,
            ok,
            value,
        }) {
            warn!("OsResult proxy.send_event failed: {error}");
        }
    }) {
        let message = format!("native OS call rejected: {error}");
        warn!("{message}");
        let _ = fallback_proxy.send_event(UserEvent::OsResult {
            label: fallback_label,
            id: fallback_id,
            ok: false,
            value: Value::String(message),
        });
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_string())
        })
        .unwrap_or_else(|| "<non-string panic>".to_string())
}

#[cfg(test)]
mod tests;
