use std::sync::Arc;

use crossbeam_channel::Sender;
#[cfg(feature = "api-single-instance")]
use serde_json::json;
use serde_json::Value;
use tao::{event_loop::EventLoopProxy, window::Window};

use tynd_protocol::runtime::{forwards_os_event, BackendCall, BackendConfig, BackendEvent};

use crate::webview::shim;
use crate::{
    api,
    api::{menu, tray},
    webview::scheme,
};

use super::{dispatch_support::smoke_duration_ms, os_event_queue::OsEventQueue, UserEvent};
use crate::webview::navigation::looks_like_url;

pub(super) fn spawn_background(name: &'static str, task: impl FnOnce() + Send + 'static) -> bool {
    match std::thread::Builder::new().name(name.into()).spawn(task) {
        Ok(_) => true,
        Err(error) => {
            crate::tynd_error!("failed to spawn background thread '{name}': {error}");
            false
        },
    }
}

pub(super) fn configure_startup_security(config: &BackendConfig) {
    if let Some(policy) = config.security.as_ref() {
        if let Err(error) = crate::security::policy::configure(policy) {
            panic!("invalid startup security policy: {error}");
        }
    }
    if let Some(capabilities) = config.capabilities.clone() {
        if let Err(error) = crate::security::acl::authority().load_value(capabilities) {
            panic!("invalid startup capability policy: {error}");
        }
    }
}

/// Window events also go to the backend, where `getWindow(label).on*`
/// handlers subscribe to them.
fn forward_to_backend(call_tx: &Sender<BackendCall>, name: &str, data: &Value) {
    if forwards_os_event(name) {
        let _ = call_tx.send(BackendCall::OsEvent {
            name: name.to_owned(),
            data: data.clone(),
        });
    }
}

pub(super) fn install_os_event_emitter(
    queue: &Arc<OsEventQueue>,
    proxy: &EventLoopProxy<UserEvent>,
    call_tx: &Sender<BackendCall>,
) {
    let emitter_queue = Arc::clone(queue);
    let emitter_proxy = proxy.clone();
    let emitter_call_tx = call_tx.clone();
    crate::ipc::events::set_emitter(Box::new(move |name, data| {
        forward_to_backend(&emitter_call_tx, name, data);
        let script = shim::eval_os_event(name, data);
        if emitter_queue.enqueue(name, script)
            && emitter_proxy.send_event(UserEvent::FlushOsEvents).is_err()
        {
            emitter_queue.cancel_schedule();
        }
    }))
    .expect("event emitter must only be registered once");
    let scoped_queue = Arc::clone(queue);
    let scoped_proxy = proxy.clone();
    let scoped_call_tx = call_tx.clone();
    crate::ipc::events::set_scoped_emitter(Box::new(move |name, data, scope| {
        forward_to_backend(&scoped_call_tx, name, data);
        let script = shim::eval_os_event(name, data);
        if scoped_queue.enqueue_scoped(name, script, scope)
            && scoped_proxy.send_event(UserEvent::FlushOsEvents).is_err()
        {
            scoped_queue.cancel_schedule();
        }
    }))
    .expect("scoped event emitter must only be registered once");
}

pub(super) fn install_host_integrations(proxy: &EventLoopProxy<UserEvent>) {
    let shutdown_proxy = proxy.clone();
    crate::cleanup::set_shutdown_hook(move || {
        let _ = shutdown_proxy.send_event(UserEvent::ForceExit);
    });

    let monitor_proxy = proxy.clone();
    api::window_cmd::set_monitor_change_hook(Box::new(move || {
        let _ = monitor_proxy.send_event(UserEvent::MonitorsChanged);
    }));

    let window_proxy = proxy.clone();
    api::window_cmd::set_backend_window_hook(Box::new(move |method, args| {
        let (reply, answer) = crossbeam_channel::bounded(1);
        window_proxy
            .send_event(UserEvent::BackendWindowCmd {
                method: method.to_owned(),
                args,
                reply,
            })
            .map_err(|_| format!("window.{method}: the event loop has stopped"))?;
        answer
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|_| format!("window.{method}: no answer from the event loop within 30s"))?
    }));

    #[cfg(feature = "plugins")]
    crate::plugins::install_default_emit();
}

#[cfg(feature = "debug-protocol")]
pub(super) fn install_debug_hooks(
    enabled: bool,
    call_tx: &Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    if !enabled {
        return;
    }
    crate::debug::install_ipc_replay(call_tx.clone());
    let eval_proxy = proxy.clone();
    crate::debug::set_eval_hook(Box::new(move |label: &str, script: &str| {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = eval_proxy.send_event(UserEvent::DebugEval {
            label: label.to_string(),
            script: script.to_string(),
            reply: tx,
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("debug eval timeout: {error}"))?
    }));

    let window_proxy = proxy.clone();
    crate::debug::set_window_hook(Box::new(move |action: &str, params: &Value| {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = window_proxy.send_event(UserEvent::DebugWindowManage {
            action: action.to_string(),
            params: params.clone(),
            reply: tx,
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("debug window manage timeout: {error}"))?
    }));
}

#[cfg(feature = "api-single-instance")]
pub(super) fn install_single_instance_hook(proxy: &EventLoopProxy<UserEvent>) {
    let proxy = proxy.clone();
    api::single_instance::set_second_launch_hook(Box::new(move |payload| {
        let _ = proxy.send_event(UserEvent::FocusPrimary);
        if let Some(argv) = payload.get("argv").and_then(Value::as_array) {
            for argument in argv.iter().skip(1).filter_map(Value::as_str) {
                if looks_like_url(argument) {
                    crate::ipc::events::emit_scoped(
                        "app:open-url",
                        &json!({ "url": argument }),
                        crate::ipc::events::EventScope::Local,
                    );
                }
            }
        }
        super::runtime::handle_second_launch(payload);
    }));
}

pub(super) fn initial_open_urls() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|argument| looks_like_url(argument))
        .collect()
}

pub(super) fn start_backend_event_relay(
    event_rx: crossbeam_channel::Receiver<BackendEvent>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    let proxy = proxy.clone();
    assert!(
        spawn_background("tynd-backend-events", move || {
            while let Ok(event) = event_rx.recv() {
                let _ = proxy.send_event(UserEvent::Backend(event));
            }
        }),
        "failed to start required backend event relay"
    );
}

pub(super) fn warm_frontend_assets(frontend_dir: Option<&str>) {
    let Some(directory) = frontend_dir.map(str::to_string) else {
        return;
    };
    let _ = spawn_background("tynd-asset-warmup", move || {
        if let Err(error) = scheme::warm(&directory) {
            crate::tynd_error!("frontend asset warmup failed: {error}");
        }
    });
}

/// Forward native menu activations (menu bar, context and tray menus all share
/// muda's global handler) to the renderer as `menu:action`.
#[cfg(any(feature = "api-menu", feature = "api-tray"))]
fn install_menu_event_handler() {
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let id = event.id().0.clone();
        let (mut payload, menu_claims) = menu::action_payload(&id);
        // A tray menu item also names its tray (and keeps its radio group exclusive).
        let trays = tray::on_menu_activated(&id);
        if menu_claims + trays.len() > 1 {
            warn_ambiguous_menu_id(&id);
        }
        if let (Some(tray), Some(fields)) = (trays.into_iter().next(), payload.as_object_mut()) {
            fields.insert("tray".into(), tray.into());
        }
        crate::ipc::events::emit("menu:action", &payload);
    }));
}

/// muda reports a click by id only: a plain item id present in several menus
/// (application bar, window bars, Dock, trays, context menu) cannot be
/// attributed to the menu it came from. Warn once per id.
#[cfg(any(feature = "api-menu", feature = "api-tray"))]
fn warn_ambiguous_menu_id(id: &str) {
    thread_local! {
        static WARNED: std::cell::RefCell<std::collections::HashSet<String>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
    }
    let first = WARNED
        .try_with(|warned| warned.borrow_mut().insert(id.to_owned()))
        .unwrap_or(false);
    if first {
        crate::tynd_warn!(
            "menu item id '{id}' exists in several menus; its clicks cannot be told apart              (`tray` names the first tray holding it). Give every item an id unique across              the application bar, window bars, the Dock, trays and context menus."
        );
    }
}

#[cfg(feature = "api-menu")]
pub(super) fn build_app_menu(
    config: &BackendConfig,
    native_window: &Window,
) -> Option<menu::AppMenu> {
    install_menu_event_handler();
    if config.menu.is_empty() {
        return None;
    }
    match menu::build_bar(&config.menu) {
        Ok(menu) => {
            menu::init_bar(&menu, native_window);
            Some(menu)
        },
        Err(error) => {
            crate::tynd_warn!("Menu build failed: {error}");
            None
        },
    }
}

/// Without `api-menu` no menu bar is built; a configured `menu` already failed
/// startup in [`ensure_static_apis_compiled`].
#[cfg(not(feature = "api-menu"))]
pub(super) fn build_app_menu(
    _config: &BackendConfig,
    _native_window: &Window,
) -> Option<menu::AppMenu> {
    #[cfg(feature = "api-tray")]
    install_menu_event_handler();
    None
}

/// The tray handle (with the configured `main` icon, if any). Icons can also
/// be created at runtime, so the handle exists whenever `api-tray` is built.
#[cfg(feature = "api-tray")]
pub(super) fn build_system_tray(
    config: &BackendConfig,
    proxy: &EventLoopProxy<UserEvent>,
) -> Option<tray::SystemTray> {
    Some(tray::init(config.tray.as_ref(), proxy))
}

/// Without `api-tray` no tray is created; a configured `tray` already failed
/// startup in [`ensure_static_apis_compiled`].
#[cfg(not(feature = "api-tray"))]
pub(super) fn build_system_tray(
    config: &BackendConfig,
    proxy: &EventLoopProxy<UserEvent>,
) -> Option<tray::SystemTray> {
    tray::init(config.tray.as_ref(), proxy);
    None
}

/// A static `menu` / `tray` config whose API is compiled out of this host fails
/// startup (fail closed) with the `api_disabled` message naming the plugin to
/// add, instead of starting an app silently missing it.
pub(super) fn ensure_static_apis_compiled(config: &BackendConfig) -> Result<(), String> {
    require_compiled("menu", !config.menu.is_empty(), cfg!(feature = "api-menu"))?;
    require_compiled("tray", config.tray.is_some(), cfg!(feature = "api-tray"))
}

fn require_compiled(api: &str, configured: bool, compiled: bool) -> Result<(), String> {
    if configured && !compiled {
        return Err(format!("`{api}` is configured: {}", api::api_disabled(api)));
    }
    Ok(())
}

pub(super) fn schedule_smoke_exit(
    smoke_mode: bool,
    native_window: &Window,
    proxy: &EventLoopProxy<UserEvent>,
) {
    if !smoke_mode {
        return;
    }
    native_window.set_visible(true);
    let proxy = proxy.clone();
    let fallback_proxy = proxy.clone();
    let duration_ms = smoke_duration_ms(std::env::var("TYND_SMOKE_DURATION_MS").ok().as_deref());
    if !spawn_background("tynd-smoke-timeout", move || {
        std::thread::sleep(std::time::Duration::from_millis(duration_ms));
        let _ = proxy.send_event(UserEvent::ForceExit);
    }) {
        let _ = fallback_proxy.send_event(UserEvent::ForceExit);
    }
}

#[cfg(test)]
mod static_api_tests {
    use super::require_compiled;

    #[test]
    fn compiled_out_static_api_fails_closed_naming_its_plugin() {
        let error = require_compiled("tray", true, false).unwrap_err();
        assert!(error.contains("`tray` is configured"));
        assert!(error.contains("`@tynd/plugin-tray`"));
        let error = require_compiled("menu", true, false).unwrap_err();
        assert!(error.contains("`@tynd/plugin-menu`"));
    }

    #[test]
    fn compiled_or_unconfigured_static_api_is_accepted() {
        assert_eq!(require_compiled("menu", true, true), Ok(()));
        assert_eq!(require_compiled("menu", false, false), Ok(()));
    }
}
