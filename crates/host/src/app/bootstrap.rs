use std::sync::Arc;

#[cfg(feature = "debug-protocol")]
use crossbeam_channel::Sender;
#[cfg(any(
    feature = "api-menu",
    feature = "api-tray",
    feature = "api-single-instance"
))]
use serde_json::json;
#[cfg(any(
    feature = "debug-protocol",
    feature = "api-tray",
    feature = "api-single-instance"
))]
use serde_json::Value;
use tao::{event_loop::EventLoopProxy, window::Window};

#[cfg(feature = "debug-protocol")]
use tynd_protocol::runtime::BackendCall;
use tynd_protocol::runtime::{BackendConfig, BackendEvent};

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

pub(super) fn install_os_event_emitter(
    queue: &Arc<OsEventQueue>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    let emitter_queue = Arc::clone(queue);
    let emitter_proxy = proxy.clone();
    crate::ipc::events::set_emitter(Box::new(move |name, data| {
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
    crate::ipc::events::set_scoped_emitter(Box::new(move |name, data, scope| {
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
        crate::ipc::events::emit("menu:action", &json!({ "id": id }));
    }));
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

#[cfg(feature = "api-tray")]
pub(super) fn build_system_tray(config: &BackendConfig) -> Option<tray::SystemTray> {
    let tray_config = config.tray.as_ref()?;
    match tray::build(tray_config) {
        Ok(tray) => {
            tray_icon::TrayIconEvent::set_event_handler(Some(handle_tray_event));
            Some(tray)
        },
        Err(error) => {
            crate::tynd_warn!("System tray failed: {error}");
            None
        },
    }
}

/// Without `api-tray` no tray is created; a configured `tray` already failed
/// startup in [`ensure_static_apis_compiled`].
#[cfg(not(feature = "api-tray"))]
pub(super) fn build_system_tray(_config: &BackendConfig) -> Option<tray::SystemTray> {
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

/// Event name for a tray click. The tray emits one `Click` for the press and
/// one for the release, so only the release counts: otherwise a single click
/// fires `tray:click` twice and an app toggling its window on it flickers.
#[cfg(feature = "api-tray")]
fn tray_click_name(
    button: tray_icon::MouseButton,
    state: tray_icon::MouseButtonState,
) -> Option<&'static str> {
    if state != tray_icon::MouseButtonState::Up {
        return None;
    }
    match button {
        tray_icon::MouseButton::Left => Some("tray:click"),
        tray_icon::MouseButton::Right => Some("tray:right-click"),
        tray_icon::MouseButton::Middle => None,
    }
}

#[cfg(feature = "api-tray")]
fn handle_tray_event(event: tray_icon::TrayIconEvent) {
    let (name, data) = match event {
        tray_icon::TrayIconEvent::Click {
            button,
            button_state,
            ..
        } => match tray_click_name(button, button_state) {
            Some(name) => (name, Value::Null),
            None => return,
        },
        tray_icon::TrayIconEvent::DoubleClick { .. } => ("tray:double-click", Value::Null),
        tray_icon::TrayIconEvent::Enter { position, .. } => {
            ("tray:enter", json!({ "x": position.x, "y": position.y }))
        },
        tray_icon::TrayIconEvent::Move { position, .. } => {
            ("tray:move", json!({ "x": position.x, "y": position.y }))
        },
        tray_icon::TrayIconEvent::Leave { position, .. } => {
            ("tray:leave", json!({ "x": position.x, "y": position.y }))
        },
        _ => return,
    };
    crate::ipc::events::emit(name, &data);
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
#[cfg(feature = "api-tray")]
mod tray_tests {
    use tray_icon::{MouseButton, MouseButtonState};

    use super::tray_click_name;

    #[test]
    fn tray_click_fires_once_on_button_up() {
        assert_eq!(
            tray_click_name(MouseButton::Left, MouseButtonState::Up),
            Some("tray:click")
        );
        assert_eq!(
            tray_click_name(MouseButton::Right, MouseButtonState::Up),
            Some("tray:right-click")
        );
        assert_eq!(
            tray_click_name(MouseButton::Left, MouseButtonState::Down),
            None
        );
        assert_eq!(
            tray_click_name(MouseButton::Right, MouseButtonState::Down),
            None
        );
        assert_eq!(
            tray_click_name(MouseButton::Middle, MouseButtonState::Up),
            None
        );
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
