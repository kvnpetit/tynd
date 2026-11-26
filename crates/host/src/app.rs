use std::sync::{atomic::Ordering, Arc};

use crossbeam_channel::Sender;
use serde_json::{json, Value};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Theme, Window, WindowId},
};
use wry::WebView;

use tynd_protocol::runtime::{BackendBridge, BackendCall, BackendConfig, BackendEvent};

use crate::{
    api,
    api::{menu, tray},
    webview::{navigation, shim},
    window,
};

mod backend_event;
mod bootstrap;
mod child_webview;
mod child_webview_commands;
mod close_guard;
mod dispatch_support;
mod gpu;
#[cfg(target_os = "linux")]
mod linux_webview;
mod os_event_queue;
mod primary_webview;
mod runtime;
mod secondary_window;
mod source_webview_commands;
mod user_event;
mod util;
mod window_command;
mod window_events;
mod window_messages;
mod window_state;
mod yields;

#[cfg(feature = "debug-protocol")]
use bootstrap::install_debug_hooks;
#[cfg(feature = "api-single-instance")]
use bootstrap::install_single_instance_hook;
use bootstrap::{
    build_app_menu, build_system_tray, configure_startup_security, ensure_static_apis_compiled,
    initial_open_urls, install_host_integrations, install_os_event_emitter, schedule_smoke_exit,
    spawn_background, start_backend_event_relay, warm_frontend_assets,
};
#[cfg(feature = "debug-protocol")]
use dispatch_support::debug_window_info;
use dispatch_support::{
    broadcast_script, broadcast_script_scoped, dispatch_os_reply, result_reply,
};
#[cfg(target_os = "linux")]
use linux_webview::LinuxWebviewContainer;
use primary_webview::{build_primary_webview, PrimaryWebview};
use runtime::{BackendRuntimeState, WindowRegistry};

use crate::ipc::events::EventScope;
use os_event_queue::OsEventQueue;
pub(crate) use runtime::subscribe_open_files;
pub(crate) use user_event::UserEvent;

use window_command::{handle_window_command, run_backend_window_command, WindowCommandContext};
use window_events::{window_closed, window_created};
use window_state::WindowState;

/// Reserved — secondary windows created via `multiwindow.create({ label })`
/// (`@tynd/plugin-multiwindow/client`) may not reuse it.
const PRIMARY_LABEL: &str = "main";

/// A secondary (non-primary) window with its own WebView and state tracker.
struct SecondaryEntry {
    webview: WebView,
    // Keep the context alive until after the WebView is dropped. Named
    // partitions use it on Windows/Linux; macOS uses the stable data-store id.
    _web_context: Option<Box<wry::WebContext>>,
    #[cfg(target_os = "linux")]
    webview_container: LinuxWebviewContainer,
    /// This window's own menu bar (`win.setMenu`, Windows / Linux).
    menu: menu::WindowMenu,
    // Drop the WebView and its GTK composition widgets before the native
    // window that owns them.
    window: Window,
    data_partition: Option<String>,
    state: WindowState,
    /// Whether this window opted into `os_call("window", "loadHtml", ...)`.
    /// Off by default — arbitrary HTML inherits the `__tynd__` shim and would
    /// expose the IPC surface to attacker-controlled markup otherwise.
    allow_load_html: bool,
}

struct AppRuntimeInit {
    config: BackendConfig,
    call_tx: Sender<BackendCall>,
    proxy: EventLoopProxy<UserEvent>,
    os_event_queue: Arc<OsEventQueue>,
    initial_open_urls: Vec<String>,
    native_window: Window,
    app_menu: Option<menu::AppMenu>,
    system_tray: Option<tray::SystemTray>,
    primary_webview: PrimaryWebview,
    debug: bool,
}

struct AppRuntime {
    debug: bool,
    config: BackendConfig,
    backend: BackendRuntimeState,
    os_event_queue: Arc<OsEventQueue>,
    initial_open_urls: Vec<String>,
    windows: WindowRegistry,
}

impl AppRuntime {
    fn new(init: AppRuntimeInit) -> Self {
        let AppRuntimeInit {
            config,
            call_tx,
            proxy,
            os_event_queue,
            initial_open_urls,
            native_window,
            app_menu,
            system_tray,
            primary_webview,
            debug,
        } = init;
        let backend = BackendRuntimeState::new(call_tx, proxy);
        let windows = WindowRegistry::new(native_window, app_menu, system_tray, primary_webview);

        Self {
            debug,
            config,
            backend,
            os_event_queue,
            initial_open_urls,
            windows,
        }
    }

    fn handle_event(
        &mut self,
        event: Event<'_, UserEvent>,
        target: &EventLoopWindowTarget<UserEvent>,
        control_flow: &mut ControlFlow,
    ) {
        match event {
            Event::UserEvent(event) => self.handle_user_event(event, target, control_flow),
            Event::WindowEvent {
                event, window_id, ..
            } => self.handle_window_event(event, window_id),
            // macOS only: LaunchServices file/URL hand-over and dock reopen.
            Event::Opened { urls } => runtime::handle_opened(&urls),
            Event::Reopen {
                has_visible_windows,
                ..
            } => runtime::handle_reopen(has_visible_windows),
            _ => {},
        }
    }

    fn handle_user_event(
        &mut self,
        event: UserEvent,
        target: &EventLoopWindowTarget<UserEvent>,
        control_flow: &mut ControlFlow,
    ) {
        match event {
            UserEvent::PageReady { label } => self.handle_page_ready(&label),
            UserEvent::FocusPrimary => self.focus_primary_window(),
            UserEvent::MonitorsChanged => {
                api::window_cmd::refresh_monitors(&self.windows.native_window);
            },
            UserEvent::ForceExit => self.force_exit(control_flow),
            UserEvent::ContextLost { label } => self.handle_context_lost(&label),
            UserEvent::FlushYields => self.flush_yields(),
            UserEvent::Backend(event) => self.dispatch_backend_event(event, control_flow),
            UserEvent::OsResult {
                label,
                id,
                ok,
                value,
            } => self.send_os_reply(&label, &id, ok, &value),
            UserEvent::WindowCmd {
                label,
                id,
                method,
                args,
                origin,
            } => self.dispatch_window_command(target, label, id, method, args, origin),
            UserEvent::CloseChildWebview { label } => self.close_child_webview(label),
            UserEvent::CloseSecondaryWindow { label } => {
                self.close_secondary_window(label, control_flow);
            },
            UserEvent::TrayCmd {
                label,
                id,
                method,
                args,
                origin,
            } => self.dispatch_tray_command(&label, &id, &method, &args, origin.as_deref()),
            UserEvent::TrayToggleWindow { tray, anchor } => {
                tray::toggle_attached(&tray, anchor, &|label| self.windows.window_for(label));
            },
            UserEvent::MenuCmd {
                label,
                id,
                method,
                args,
            } => self.dispatch_menu_command(&label, &id, &method, &args),
            UserEvent::FlushOsEvents => self.flush_os_events(),
            #[cfg(feature = "debug-protocol")]
            UserEvent::DebugEval {
                label,
                script,
                reply,
            } => self.handle_debug_eval(&label, &script, reply),
            #[cfg(feature = "debug-protocol")]
            UserEvent::DebugWindowManage {
                action,
                params,
                reply,
            } => self.handle_debug_window_manage(&action, &params, &reply, control_flow),
            UserEvent::ProceedClose { label } => self.proceed_close(&label, control_flow),
            UserEvent::BackendWindowCmd {
                method,
                args,
                reply,
            } => {
                let result = self.run_backend_window_command(target, &method, &args);
                let _ = reply.send(result);
            },
        }
    }

    fn handle_page_ready(&mut self, label: &str) {
        // App readiness is tied to the primary document only.
        // Secondary and child documents also carry JS_PAGE_READY,
        // but must not re-run backend lifecycle hooks.
        if label != PRIMARY_LABEL {
            return;
        }
        if window::shows_on_page_ready(&self.config.window) {
            self.windows.native_window.set_visible(true);
        }
        if self.debug {
            // Dev runs (`tynd dev` passes `--debug`) announce readiness so the
            // terminal shows when the app is usable and what it loaded.
            crate::tynd_info!(
                "window ready: {}",
                self.config
                    .dev_url
                    .as_deref()
                    .or_else(|| self
                        .config
                        .frontend_dir
                        .as_ref()
                        .map(|_| "tynd://localhost/"))
                    .unwrap_or("placeholder page (no frontend configured)")
            );
        }
        // `on_ready` runs on every primary page load: a dev backend restart
        // reloads the page and needs its new process to be told again.
        self.backend.send_lifecycle("on_ready");
        #[cfg(feature = "plugins")]
        crate::plugins::broadcast_event(&crate::plugins::HostEvent::AppReady);
        // Launch URLs are delivered once: a reload must not replay them (an
        // OAuth callback URL would be redeemed again).
        for url in std::mem::take(&mut self.initial_open_urls) {
            crate::ipc::events::emit_scoped(
                "app:open-url",
                &json!({ "url": url }),
                EventScope::Local,
            );
        }
    }

    fn focus_primary_window(&self) {
        self.windows.native_window.set_minimized(false);
        self.windows.native_window.set_visible(true);
        self.windows.native_window.set_focus();
    }

    fn force_exit(&self, control_flow: &mut ControlFlow) {
        if !self.backend.begin_exit() {
            return;
        }
        crate::cleanup::run();
        *control_flow = ControlFlow::Exit;
    }

    fn handle_context_lost(&self, label: &str) {
        // Notify the frontend so it can clear in-flight WebGPU / WebGL state,
        // then reload the affected webview to re-create the GPU contexts.
        let payload = json!({ "label": label });
        let payload_str = serde_json::to_string(&payload).unwrap_or_default();
        let emit_script = shim::eval_dispatch("webview:context-lost", &payload_str);
        let Some(webview) = self.windows.webview_for(label) else {
            crate::tynd_warn!("ignoring context-lost event for unknown webview '{label}'");
            return;
        };
        let _ = webview.evaluate_script(&emit_script);
        crate::webview::process_failed::emit(label, "crashed");
        let _ = webview.evaluate_script("window.location.reload();");
        crate::tynd_warn!("webview '{label}' context lost — reloading to recover GPU state");
    }

    fn flush_yields(&mut self) {
        // Clear before flush so arrivals during evaluation schedule a new wakeup.
        self.backend.mark_yield_flush_idle();
        self.backend.flush_yields(&self.windows);
    }

    fn dispatch_backend_event(&mut self, event: BackendEvent, control_flow: &mut ControlFlow) {
        self.backend
            .dispatch_backend_event(event, &self.windows, control_flow);
    }

    fn send_os_reply(&self, label: &str, id: &str, ok: bool, value: &Value) {
        dispatch_os_reply(
            label,
            id,
            ok,
            value,
            &self.windows.webview,
            &self.windows.secondaries,
            &self.windows.child_webviews,
        );
    }

    fn send_result_reply(&self, label: &str, id: &str, result: Result<Value, String>) {
        let (ok, value) = result_reply(result);
        self.send_os_reply(label, id, ok, &value);
    }

    fn dispatch_window_command(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        label: String,
        id: String,
        method: String,
        args: Value,
        origin: Option<String>,
    ) {
        handle_window_command(
            WindowCommandContext {
                caller_origin: origin.as_deref(),
                target,
                debug: self.debug,
                call_tx: self.backend.call_tx(),
                proxy: self.backend.proxy(),
                dev_url: self.config.dev_url.as_deref(),
                frontend_dir: self.config.frontend_dir.as_deref(),
                windows: &mut self.windows,
            },
            label,
            id,
            method,
            args,
        );
    }

    fn run_backend_window_command(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        method: &str,
        args: &Value,
    ) -> Result<Value, String> {
        run_backend_window_command(
            WindowCommandContext {
                caller_origin: None,
                target,
                debug: self.debug,
                call_tx: self.backend.call_tx(),
                proxy: self.backend.proxy(),
                dev_url: self.config.dev_url.as_deref(),
                frontend_dir: self.config.frontend_dir.as_deref(),
                windows: &mut self.windows,
            },
            method,
            args,
        )
    }

    fn close_child_webview(&mut self, label: String) {
        if let Some(entry) = self.windows.child_webviews.remove(&label) {
            let parent_label = entry.parent_label.clone();
            drop(entry);
            crate::ipc::dispatch::cancel_calls_for_label(&label, self.backend.call_tx());
            navigation::clear_remote_origin(&label);
            self.windows.zoom_levels.remove(&label);
            crate::ipc::events::emit(
                "webview:closed",
                &json!({ "label": label, "parent": parent_label }),
            );
        }
    }

    fn close_secondary_window(&mut self, label: String, control_flow: &mut ControlFlow) {
        self.teardown_secondary(&label, control_flow);
    }

    /// The one teardown path for a secondary window, shared by the
    /// `window.close` API, the user clicking X and the debug protocol. Returns
    /// `false` (and does nothing) when `label` is not a live secondary window,
    /// so a close racing another close cannot fire twice.
    fn teardown_secondary(&mut self, label: &str, control_flow: &mut ControlFlow) -> bool {
        if !self.windows.secondaries.contains_key(label) {
            return false;
        }
        crate::ipc::dispatch::cancel_calls_for_label(label, self.backend.call_tx());
        self.windows
            .remove_child_webviews_for_parent(label, self.backend.call_tx());
        if let Some(entry) = self.windows.secondaries.remove(label) {
            self.windows.window_id_to_label.remove(&entry.window.id());
            drop(entry);
        }
        navigation::clear_remote_origin(label);
        self.remove_window_preferences(label);
        crate::ipc::events::emit("window:closed", &json!({ "label": label }));
        window_closed(self.backend.call_tx(), label);
        self.exit_if_last_window_closed(control_flow);
        true
    }

    /// Quit when `quitOnLastWindowClosed` is set, no secondary window is left
    /// and the primary window is hidden.
    fn exit_if_last_window_closed(&mut self, control_flow: &mut ControlFlow) {
        if self.config.quit_on_last_window_closed
            && self.windows.secondaries.is_empty()
            && !self.windows.native_window.is_visible()
            && self.backend.begin_exit()
        {
            crate::cleanup::run();
            *control_flow = ControlFlow::Exit;
        }
    }

    fn remove_window_preferences(&mut self, label: &str) {
        self.windows.remove_window_preferences(label);
    }

    fn dispatch_tray_command(
        &self,
        label: &str,
        id: &str,
        method: &str,
        args: &Value,
        origin: Option<&str>,
    ) {
        // Child webviews drive the tray on behalf of their containing window.
        let window_label = self
            .windows
            .child_webviews
            .get(label)
            .map_or(label, |entry| entry.parent_label.as_str());
        let call = tray::TrayCall {
            caller: window_label,
            caller_origin: origin,
            window_for: &|window| self.windows.window_for(window),
        };
        let result = match self.windows.system_tray.as_ref() {
            Some(tray) => tray::dispatch(tray, &call, method, args),
            None => Err(tray::unavailable_error()),
        };
        self.send_result_reply(label, id, result);
    }

    fn dispatch_menu_command(&self, label: &str, id: &str, method: &str, args: &Value) {
        // Menu mutations use the thread-local registry populated by build_bar.
        // Child webviews pop context menus up on their containing window.
        let window_label = self
            .windows
            .child_webviews
            .get(label)
            .map_or(label, |entry| entry.parent_label.as_str());
        let target = menu::MenuTarget {
            source: self.windows.window_for(window_label),
            primary: self.windows.window_for(PRIMARY_LABEL),
        };
        self.send_result_reply(label, id, menu::dispatch(&target, method, args));
    }

    fn flush_os_events(&self) {
        let batch = self.os_event_queue.drain_batch();
        if batch.dropped > 0 {
            crate::tynd_warn!(
                "dropped {} OS/plugin event(s): renderer queue reached capacity {}",
                batch.dropped,
                os_event_queue::CAPACITY
            );
            let overflow = shim::eval_os_event(
                "tynd:event-overflow",
                &json!({
                    "dropped": batch.dropped,
                    "capacity": os_event_queue::CAPACITY,
                }),
            );
            broadcast_script(
                &overflow,
                &self.windows.webview,
                &self.windows.secondaries,
                &self.windows.child_webviews,
            );
        }
        for (script, scope) in batch.scripts {
            if scope == EventScope::All {
                broadcast_script(
                    &script,
                    &self.windows.webview,
                    &self.windows.secondaries,
                    &self.windows.child_webviews,
                );
            } else {
                broadcast_script_scoped(
                    &script,
                    &scope,
                    &self.windows.webview,
                    &self.windows.secondaries,
                    &self.windows.child_webviews,
                );
            }
        }
        if batch.reschedule
            && self
                .backend
                .proxy()
                .send_event(UserEvent::FlushOsEvents)
                .is_err()
        {
            self.os_event_queue.cancel_schedule();
        }
    }

    #[cfg(feature = "debug-protocol")]
    fn handle_debug_eval(&self, label: &str, script: &str, reply: Sender<Result<Value, String>>) {
        let Some(webview) = self.windows.webview_for(label) else {
            let _ = reply.send(Err(format!("webview '{label}' not found")));
            return;
        };
        let callback_reply = reply.clone();
        let result = webview.evaluate_script_with_callback(script, move |raw| {
            let parsed = serde_json::from_str(&raw)
                .map_err(|error| format!("decode eval result: {error}"))
                .or_else(|_| Ok(Value::String(raw.clone())));
            let _ = callback_reply.send(parsed);
        });
        if let Err(error) = result {
            let _ = reply.send(Err(format!("evaluate_script_with_callback: {error}")));
        }
    }

    #[cfg(feature = "debug-protocol")]
    fn handle_debug_window_manage(
        &mut self,
        action: &str,
        params: &Value,
        reply: &Sender<Result<Value, String>>,
        control_flow: &mut ControlFlow,
    ) {
        let response = self.debug_window_manage(action, params, control_flow);
        let _ = reply.send(response);
    }

    #[cfg(feature = "debug-protocol")]
    fn debug_window_manage(
        &mut self,
        action: &str,
        params: &Value,
        control_flow: &mut ControlFlow,
    ) -> Result<Value, String> {
        let label = params
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or(PRIMARY_LABEL)
            .to_string();
        if action == "list" {
            let mut windows = Vec::with_capacity(self.windows.secondaries.len() + 1);
            windows.push(debug_window_info(
                PRIMARY_LABEL,
                &self.windows.native_window,
            ));
            windows.extend(
                self.windows
                    .secondaries
                    .iter()
                    .map(|(label, entry)| debug_window_info(label, &entry.window)),
            );
            return Ok(Value::Array(windows));
        }
        if action == "info" {
            let window = self.debug_window(&label)?;
            return Ok(debug_window_info(&label, window));
        }
        if action == "close" {
            return self.debug_close_window(&label, control_flow);
        }
        let window = self.debug_window(&label)?;
        debug_mutate_window(window, action, params)
    }

    #[cfg(feature = "debug-protocol")]
    fn debug_window(&self, label: &str) -> Result<&Window, String> {
        self.windows
            .window_for(label)
            .ok_or_else(|| format!("window '{label}' not found"))
    }

    #[cfg(feature = "debug-protocol")]
    fn debug_close_window(
        &mut self,
        label: &str,
        control_flow: &mut ControlFlow,
    ) -> Result<Value, String> {
        if label == PRIMARY_LABEL {
            let proxy = self.backend.proxy().clone();
            let fallback_proxy = proxy.clone();
            if !spawn_background("tynd-debug-close", move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                let _ = proxy.send_event(UserEvent::ForceExit);
            }) {
                let _ = fallback_proxy.send_event(UserEvent::ForceExit);
            }
            return Ok(json!({ "ok": true }));
        }
        if self.teardown_secondary(label, control_flow) {
            Ok(json!({ "ok": true }))
        } else {
            Err(format!("window '{label}' not found"))
        }
    }

    fn handle_window_event(&mut self, event: WindowEvent<'_>, window_id: WindowId) {
        match event {
            WindowEvent::CloseRequested => self.handle_close_requested(window_id),
            WindowEvent::Resized(size) => self.handle_resized(window_id, size),
            WindowEvent::Moved(position) => self.handle_moved(window_id, position),
            WindowEvent::Focused(focused) => self.handle_focused(window_id, focused),
            WindowEvent::KeyboardInput {
                event: key_event, ..
            } if key_event.physical_key == tao::keyboard::KeyCode::Escape
                && key_event.state == tao::event::ElementState::Pressed =>
            {
                self.handle_escape(window_id);
            },
            WindowEvent::ThemeChanged(theme) => self.handle_theme_changed(window_id, theme),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.handle_scale_factor_changed(window_id, scale_factor);
            },
            _ => {},
        }
    }

    fn handle_close_requested(&self, window_id: WindowId) {
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        self.request_close(label);
    }

    /// Every window's close is preventable: emit `window:close-requested`
    /// (renderers and backend handlers), then close after a 500ms grace
    /// period unless a handler called `cancelClose()` for this window.
    fn request_close(&self, label: String) {
        if !close_guard::begin(&label) {
            return;
        }
        crate::ipc::events::emit("window:close-requested", &json!({ "label": label }));

        let proxy = self.backend.proxy().clone();
        let fallback_proxy = proxy.clone();
        let fallback_label = label.clone();
        if !spawn_background("tynd-close-grace", move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = proxy.send_event(UserEvent::ProceedClose { label });
        }) {
            let _ = fallback_proxy.send_event(UserEvent::ProceedClose {
                label: fallback_label,
            });
        }
    }

    /// The grace period of `label` ended.
    fn proceed_close(&mut self, label: &str, control_flow: &mut ControlFlow) {
        if !close_guard::finish(label) {
            return;
        }
        if label != PRIMARY_LABEL {
            self.teardown_secondary(label, control_flow);
            return;
        }
        self.windows.native_window.set_visible(false);
        self.backend.send_lifecycle("on_close");
        let proxy = self.backend.proxy().clone();
        let fallback_proxy = proxy.clone();
        let exit_started = Arc::clone(self.backend.exit_started());
        if !spawn_background("tynd-close-watchdog", move || {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if !exit_started.load(Ordering::SeqCst) {
                let _ = proxy.send_event(UserEvent::ForceExit);
            }
        }) {
            let _ = fallback_proxy.send_event(UserEvent::ForceExit);
        }
    }

    fn handle_resized(&mut self, window_id: WindowId, size: dpi::PhysicalSize<u32>) {
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        crate::ipc::events::emit(
            "window:resized",
            &json!({ "label": &label, "width": size.width, "height": size.height }),
        );
        if window_id == self.windows.primary_id {
            self.windows
                .last_state
                .diff_and_emit(&self.windows.native_window, &label);
        } else if let Some(entry) = self.windows.secondaries.get_mut(&label) {
            let SecondaryEntry { window, state, .. } = entry;
            state.diff_and_emit(window, &label);
        }
        self.resize_child_webviews(&label);
    }

    fn resize_child_webviews(&mut self, label: &str) {
        self.windows.resize_child_webviews(label);
    }

    fn handle_moved(&self, window_id: WindowId, position: dpi::PhysicalPosition<i32>) {
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        crate::ipc::events::emit(
            "window:moved",
            &json!({ "label": label, "x": position.x, "y": position.y }),
        );
    }

    fn handle_focused(&self, window_id: WindowId, focused: bool) {
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        crate::ipc::events::emit(
            if focused {
                "window:focused"
            } else {
                "window:blurred"
            },
            &json!({ "label": label }),
        );
        #[cfg(feature = "plugins")]
        crate::plugins::broadcast_event(&crate::plugins::HostEvent::WindowFocus {
            label: label.clone(),
            focused,
        });
        if !focused
            && self
                .windows
                .hide_on_focus_lost
                .get(&label)
                .copied()
                .unwrap_or(false)
        {
            if let Some(entry) = self.windows.secondaries.get(&label) {
                entry.window.set_visible(false);
            }
        }
        if !focused {
            tray::on_window_blurred(&label, self.windows.window_for(&label));
        }
    }

    fn handle_escape(&self, window_id: WindowId) {
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        if self
            .windows
            .hide_on_escape
            .get(&label)
            .copied()
            .unwrap_or(false)
        {
            if let Some(entry) = self.windows.secondaries.get(&label) {
                entry.window.set_visible(false);
            }
        }
    }

    fn handle_theme_changed(&self, window_id: WindowId, theme: Theme) {
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        let name = if matches!(theme, Theme::Dark) {
            "dark"
        } else {
            "light"
        };
        crate::ipc::events::emit(
            "window:theme-changed",
            &json!({ "label": label, "theme": name }),
        );
    }

    fn handle_scale_factor_changed(&self, window_id: WindowId, scale_factor: f64) {
        api::window_cmd::refresh_monitors(&self.windows.native_window);
        let Some(label) = self.windows.label_for(window_id) else {
            return;
        };
        crate::ipc::events::emit(
            "window:dpi-changed",
            &json!({ "label": label, "scale": scale_factor }),
        );
    }
}

#[cfg(feature = "debug-protocol")]
fn debug_mutate_window(window: &Window, action: &str, params: &Value) -> Result<Value, String> {
    match action {
        "focus" => window.set_focus(),
        "minimize" => window.set_minimized(true),
        "unminimize" => window.set_minimized(false),
        "maximize" => window.set_maximized(true),
        "unmaximize" => window.set_maximized(false),
        "show" => window.set_visible(true),
        "hide" => window.set_visible(false),
        "move" => {
            let x = params.get("x").and_then(Value::as_i64).ok_or("missing x")?;
            let y = params.get("y").and_then(Value::as_i64).ok_or("missing y")?;
            window.set_outer_position(dpi::PhysicalPosition::new(x as i32, y as i32));
        },
        "resize" => {
            let width = params
                .get("width")
                .and_then(Value::as_u64)
                .ok_or("missing width")? as u32;
            let height = params
                .get("height")
                .and_then(Value::as_u64)
                .ok_or("missing height")? as u32;
            window.set_inner_size(dpi::PhysicalSize::new(width, height));
        },
        other => return Err(format!("unknown action: {other}")),
    }
    Ok(json!({ "ok": true }))
}

/// Log a fatal startup error and exit. `process::exit` skips destructors, so
/// run the cleanup (plugin shutdown, temp dirs, Sentry flush) first.
pub(crate) fn exit_startup_failure(error: &str) -> ! {
    crate::tynd_error!("{error}");
    crate::cleanup::run();
    std::process::exit(1);
}

pub fn run_app(bridge: BackendBridge, debug: bool) -> ! {
    let BackendBridge {
        config,
        call_tx,
        event_rx,
    } = bridge;
    let smoke_mode = config.smoke_mode;

    // GPU environment must exist before any WebView backend initializes, and
    // `set_var` must run before this crate spawns its own threads (debug
    // server, signal handler, relays below).
    gpu::apply_pre_webview_env(&config.window);

    if let Err(error) = ensure_static_apis_compiled(&config) {
        exit_startup_failure(&error);
    }
    configure_startup_security(&config);
    navigation::set_dev_origin(config.dev_url.as_deref());
    if let Err(error) = crate::webview::protocols::configure(&config, &call_tx) {
        exit_startup_failure(&error);
    }

    // Debug protocol — bind UDS / TCP loopback when enabled.
    #[cfg(feature = "debug-protocol")]
    let debug_protocol_enabled = crate::debug::start_if_enabled().is_some();
    #[cfg(not(feature = "debug-protocol"))]
    let debug_protocol_enabled = false;

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let os_event_queue = Arc::new(OsEventQueue::new());

    install_os_event_emitter(&os_event_queue, &proxy, &call_tx);
    install_host_integrations(&proxy);
    #[cfg(feature = "debug-protocol")]
    install_debug_hooks(debug_protocol_enabled, &call_tx, &proxy);
    #[cfg(feature = "api-single-instance")]
    install_single_instance_hook(&proxy);

    let initial_open_urls = initial_open_urls();
    start_backend_event_relay(event_rx, &proxy);
    // A dev server serves the frontend; the on-disk directory is not read (and
    // usually does not exist yet), so warming it would only log a spurious error.
    if config.dev_url.is_none() {
        warm_frontend_assets(config.frontend_dir.as_deref());
    }

    let native_window =
        window::build_window(&config.window, config.icon_path.as_deref(), &event_loop);
    let app_menu = build_app_menu(&config, &native_window);
    let system_tray = build_system_tray(&config, &proxy);
    let primary_webview = build_primary_webview(
        &config,
        &native_window,
        &call_tx,
        &proxy,
        debug,
        debug_protocol_enabled,
    )
    .unwrap_or_else(|error| exit_startup_failure(&error));

    schedule_smoke_exit(smoke_mode, &native_window, &proxy);
    gpu::warn_if_fps_above_monitor(&config.window, &native_window);

    window_created(&call_tx, PRIMARY_LABEL);

    let mut runtime = AppRuntime::new(AppRuntimeInit {
        config,
        call_tx,
        proxy,
        os_event_queue,
        initial_open_urls,
        native_window,
        app_menu,
        system_tray,
        primary_webview,
        debug,
    });

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        runtime.handle_event(event, target, control_flow);
    })
}
