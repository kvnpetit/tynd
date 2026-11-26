use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use crossbeam_channel::Sender;
use tao::{
    event_loop::EventLoopProxy,
    window::{Window, WindowId},
};
use wry::WebView;

#[cfg(target_os = "linux")]
use super::linux_webview::LinuxWebviewContainer;
use super::{
    backend_event::{handle_backend_event, BackendEventContext},
    bootstrap::spawn_background,
    child_webview::{update_auto_resize, ChildWebviewEntry, ChildWebviewParent},
    dispatch_support::{label_for, remove_child_webviews_for_parent, window_for_label},
    primary_webview::PrimaryWebview,
    window_state::WindowState,
    yields::{flush_yields, webview_for, YieldBucket, YIELD_BATCH_MAX, YIELD_FLUSH_MS},
    SecondaryEntry, UserEvent, PRIMARY_LABEL,
};
use tynd_protocol::runtime::{BackendCall, BackendEvent};

mod open_files;

/// Handle tao `Event::Opened`: macOS documents and custom-scheme URLs handed
/// over by LaunchServices. Files go to `app:open-file` (buffered until the
/// frontend subscribes), other URLs to the existing `app:open-url` event.
pub(super) fn handle_opened(urls: &[url::Url]) {
    let (files, others) = open_files::split_opened(urls);
    open_files::deliver_files(files);
    open_files::deliver_urls(&others);
}

/// Handle tao `Event::Reopen` (macOS dock click or relaunch of a running app).
pub(super) fn handle_reopen(has_visible_windows: bool) {
    crate::ipc::events::emit(
        "app:reopen",
        &serde_json::json!({ "hasVisibleWindows": has_visible_windows }),
    );
}

/// Windows / Linux: a second launch forwards `{ argv, cwd }`; surface the file
/// paths in it as `app:open-file`, resolved against the forwarded `cwd`.
#[cfg(feature = "api-single-instance")]
pub(super) fn handle_second_launch(payload: &serde_json::Value) {
    let Some(argv) = payload.get("argv").and_then(serde_json::Value::as_array) else {
        return;
    };
    let cwd = payload
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .filter(|cwd| !cwd.is_empty())
        .map(std::path::Path::new);
    let args = argv
        .iter()
        .skip(1)
        .filter_map(serde_json::Value::as_str)
        .map(str::to_owned);
    open_files::deliver_files(open_files::argv_file_paths(
        args,
        cwd,
        std::path::Path::is_file,
    ));
}

/// Backs `app.subscribeOpenFiles`: returns `{ paths }` buffered since launch.
pub(crate) fn subscribe_open_files() -> serde_json::Value {
    open_files::subscribe()
}

/// Backend transport and buffered-delivery state owned by the app event loop.
pub(super) struct BackendRuntimeState {
    call_tx: Sender<BackendCall>,
    proxy: EventLoopProxy<UserEvent>,
    exit_started: Arc<AtomicBool>,
    yield_buffer: HashMap<String, YieldBucket>,
    flush_scheduled: Arc<AtomicBool>,
}

impl BackendRuntimeState {
    pub(super) fn new(call_tx: Sender<BackendCall>, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            call_tx,
            proxy,
            exit_started: Arc::new(AtomicBool::new(false)),
            yield_buffer: HashMap::new(),
            flush_scheduled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn call_tx(&self) -> &Sender<BackendCall> {
        &self.call_tx
    }

    pub(super) fn proxy(&self) -> &EventLoopProxy<UserEvent> {
        &self.proxy
    }

    pub(super) fn exit_started(&self) -> &Arc<AtomicBool> {
        &self.exit_started
    }

    pub(super) fn begin_exit(&self) -> bool {
        !self.exit_started.swap(true, Ordering::SeqCst)
    }

    pub(super) fn send_lifecycle(&self, name: &str) {
        let _ = self.call_tx.send(BackendCall::lifecycle(name));
    }

    pub(super) fn mark_yield_flush_idle(&self) {
        self.flush_scheduled.store(false, Ordering::Release);
    }

    pub(super) fn push_yield(&mut self, label: String, id: String, value: String) -> bool {
        let bucket = self.yield_buffer.entry(label).or_default();
        bucket.push((id, value));
        bucket.len() >= YIELD_BATCH_MAX
    }

    pub(super) fn flush_yields(&mut self, windows: &WindowRegistry) {
        flush_yields(
            &mut self.yield_buffer,
            &windows.webview,
            &windows.secondaries,
            &windows.child_webviews,
        );
    }

    pub(super) fn clear_yields(&mut self) {
        self.yield_buffer.clear();
    }

    pub(super) fn schedule_yield_flush(&self) {
        if self.flush_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let proxy = self.proxy.clone();
        let fallback_proxy = proxy.clone();
        let fallback_latch = Arc::clone(&self.flush_scheduled);
        if !spawn_background("tynd-yield-flush", move || {
            std::thread::sleep(std::time::Duration::from_millis(YIELD_FLUSH_MS));
            let _ = proxy.send_event(UserEvent::FlushYields);
        }) {
            fallback_latch.store(false, Ordering::Release);
            let _ = fallback_proxy.send_event(UserEvent::FlushYields);
        }
    }

    pub(super) fn dispatch_backend_event(
        &mut self,
        event: BackendEvent,
        windows: &WindowRegistry,
        control_flow: &mut tao::event_loop::ControlFlow,
    ) {
        handle_backend_event(
            event,
            BackendEventContext {
                backend: self,
                windows,
                control_flow,
            },
        );
    }
}

/// Owns every native window/webview resource and its label-indexed UI state.
/// Field order preserves the required child-before-parent destruction order.
pub(super) struct WindowRegistry {
    pub(super) webview_app_name: String,
    pub(super) webview: WebView,
    _web_context: wry::WebContext,
    #[cfg(target_os = "linux")]
    pub(super) primary_webview_container: LinuxWebviewContainer,
    pub(super) child_webviews: HashMap<String, ChildWebviewEntry>,
    pub(super) secondaries: HashMap<String, SecondaryEntry>,
    _app_menu: Option<crate::api::menu::AppMenu>,
    pub(super) system_tray: Option<crate::api::tray::SystemTray>,
    pub(super) native_window: Window,
    pub(super) last_state: WindowState,
    pub(super) window_id_to_label: HashMap<WindowId, String>,
    pub(super) primary_id: WindowId,
    pub(super) zoom_levels: HashMap<String, f64>,
    pub(super) hide_on_focus_lost: HashMap<String, bool>,
    pub(super) hide_on_escape: HashMap<String, bool>,
}

impl WindowRegistry {
    pub(super) fn new(
        native_window: Window,
        app_menu: Option<crate::api::menu::AppMenu>,
        system_tray: Option<crate::api::tray::SystemTray>,
        primary_webview: PrimaryWebview,
    ) -> Self {
        let PrimaryWebview {
            app_name: webview_app_name,
            webview,
            _web_context: web_context,
            #[cfg(target_os = "linux")]
                container: primary_webview_container,
        } = primary_webview;
        let last_state = WindowState::capture(&native_window);
        let primary_id = native_window.id();
        let mut window_id_to_label = HashMap::new();
        window_id_to_label.insert(primary_id, PRIMARY_LABEL.into());

        Self {
            webview_app_name,
            webview,
            _web_context: web_context,
            #[cfg(target_os = "linux")]
            primary_webview_container,
            child_webviews: HashMap::new(),
            secondaries: HashMap::new(),
            _app_menu: app_menu,
            system_tray,
            native_window,
            last_state,
            window_id_to_label,
            primary_id,
            zoom_levels: HashMap::new(),
            hide_on_focus_lost: HashMap::new(),
            hide_on_escape: HashMap::new(),
        }
    }

    pub(super) fn webview_for(&self, label: &str) -> Option<&WebView> {
        webview_for(
            label,
            &self.webview,
            &self.secondaries,
            &self.child_webviews,
        )
    }

    pub(super) fn window_for(&self, label: &str) -> Option<&Window> {
        window_for_label(label, &self.native_window, &self.secondaries)
    }

    pub(super) fn label_for(&self, window_id: WindowId) -> Option<String> {
        label_for(window_id, &self.window_id_to_label)
    }

    pub(super) fn resize_child_webviews(&mut self, label: &str) {
        let parent = if label == PRIMARY_LABEL {
            &self.native_window
        } else {
            let Some(entry) = self.secondaries.get(label) else {
                return;
            };
            &entry.window
        };
        #[cfg(target_os = "linux")]
        let parent_container = if label == PRIMARY_LABEL {
            &self.primary_webview_container
        } else {
            &self
                .secondaries
                .get(label)
                .expect("live secondary window must have a GTK WebView container")
                .webview_container
        };

        for entry in self
            .child_webviews
            .values_mut()
            .filter(|entry| entry.parent_label == label)
        {
            update_auto_resize(
                entry,
                ChildWebviewParent {
                    window: parent,
                    #[cfg(target_os = "linux")]
                    container: parent_container,
                },
            );
        }
    }

    pub(super) fn remove_window_preferences(&mut self, label: &str) {
        self.hide_on_focus_lost.remove(label);
        self.hide_on_escape.remove(label);
        self.zoom_levels.remove(label);
    }

    pub(super) fn remove_child_webviews_for_parent(
        &mut self,
        label: &str,
        call_tx: &Sender<BackendCall>,
    ) {
        remove_child_webviews_for_parent(label, &mut self.child_webviews, call_tx);
    }
}
