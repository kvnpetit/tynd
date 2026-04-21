use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget},
    window::{Theme, Window, WindowBuilder, WindowId},
};
use wry::{http::Request, DragDropEvent, PageLoadEvent, WebView, WebViewBuilder};

use crate::{
    ipc, menu, os,
    runtime::{BackendBridge, BackendCall, BackendEvent},
    scheme, scheme_bin, tray, window,
};

mod dispatch;
mod util;

use util::webview_data_dir;

/// Label used internally for the app's first (primary) window. Secondary
/// windows created via `tyndWindow.create({ label })` cannot reuse it.
const PRIMARY_LABEL: &str = "main";

#[derive(Debug)]
enum UserEvent {
    Backend(BackendEvent),
    PageReady,
    OsResult {
        /// Label of the window that issued the os_call — the result is
        /// evaluated back on that webview only.
        label: String,
        id: String,
        ok: bool,
        value: Value,
    },
    /// Pre-serialized JS snippet for an OS event — saves a `Value.clone()` per
    /// emit on hot paths like terminal:data and websocket:message.
    OsEventScript(String),
    WindowCmd {
        label: String,
        id: String,
        method: String,
        args: Value,
    },
    TrayCmd {
        label: String,
        id: String,
        method: String,
        args: Value,
    },
    /// Sent by a timeout thread when on_close takes too long
    ForceExit,
    /// Sent 500ms after CloseRequested if `cancelClose()` wasn't called.
    ProceedClose,
    /// Sent by the single-instance listener when a duplicate launch forwards
    /// its argv — we bring the primary window to the front.
    FocusPrimary,
    /// Sent 10ms after the first buffered yield (or synchronously when the
    /// buffer exceeds `YIELD_BATCH_MAX`) to flush pending chunks to every
    /// webview in one `evaluate_script` per window.
    FlushYields,
}

/// Maximum yields buffered per window before we flush synchronously.
/// Higher = fewer `evaluate_script` calls, lower = smaller memory spike.
const YIELD_BATCH_MAX: usize = 64;
/// How long the first buffered yield waits for siblings before flushing.
/// Invisible to users (<1 frame) but cuts per-chunk overhead on bursty streams.
const YIELD_FLUSH_MS: u64 = 10;

/// A secondary (non-primary) window with its own WebView and state tracker.
struct SecondaryEntry {
    window: Window,
    webview: WebView,
    state: WindowState,
}

pub fn run_app(bridge: BackendBridge, debug: bool) -> ! {
    let BackendBridge {
        config,
        call_tx,
        event_rx,
    } = bridge;

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    {
        let proxy = proxy.clone();
        os::events::set_emitter(Box::new(move |name, data| {
            // Serialize once here (background thread) so the main event
            // loop just passes the string to the WebView.
            let script = ipc::eval_os_event(name, data);
            let _ = proxy.send_event(UserEvent::OsEventScript(script));
        }));
    }

    // Global hotkeys are emitted once registered; wire the handler at startup
    // so the app doesn't miss events registered before the first user call.
    os::shortcuts::install_event_handler();

    {
        // When another instance of this app forwards its argv via the
        // single-instance socket, bring the primary window to the front and
        // emit `app:open-url` for any URL-looking argument.
        let proxy = proxy.clone();
        os::single_instance::set_second_launch_hook(Box::new(move |payload| {
            let _ = proxy.send_event(UserEvent::FocusPrimary);
            if let Some(argv) = payload.get("argv").and_then(Value::as_array) {
                for arg in argv.iter().skip(1) {
                    if let Some(s) = arg.as_str() {
                        if looks_like_url(s) {
                            os::events::emit("app:open-url", &json!({ "url": s }));
                        }
                    }
                }
            }
        }));
    }

    // URLs present in argv on cold start (user double-clicked a deep link).
    // Deferred until PageReady so frontend handlers actually exist.
    let initial_open_urls: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| looks_like_url(a))
        .collect();

    // Relay BackendEvents -> tao UserEvents
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            while let Ok(evt) = event_rx.recv() {
                let _ = proxy.send_event(UserEvent::Backend(evt));
            }
        });
    }

    // Pre-warm asset cache in background so the first WebView request is instant.
    if let Some(ref dir) = config.frontend_dir {
        let dir = dir.clone();
        std::thread::spawn(move || scheme::warm(&dir));
    }

    // Build native window
    let native_window =
        window::build_window(&config.window, config.icon_path.as_deref(), &event_loop);

    // muda menu event handler (shared by app menu AND tray menu)
    {
        let proxy = proxy.clone();
        muda::MenuEvent::set_event_handler(Some(move |evt: muda::MenuEvent| {
            let id = evt.id().0.clone();
            let data = serde_json::json!({ "id": id });
            let _ = proxy.send_event(UserEvent::OsEventScript(ipc::eval_os_event(
                "menu:action",
                &data,
            )));
        }));
    }

    let _app_menu: Option<muda::Menu> = if config.menu.is_empty() {
        None
    } else {
        match menu::build_bar(&config.menu) {
            Ok(m) => {
                menu::init_bar(&m, &native_window);
                Some(m)
            },
            Err(e) => {
                crate::tynd_log!("Menu build failed: {e}");
                None
            },
        }
    };

    let system_tray: Option<tray_icon::TrayIcon> =
        config.tray.as_ref().and_then(|tc| match tray::build(tc) {
            Ok(tray) => {
                let proxy = proxy.clone();
                tray_icon::TrayIconEvent::set_event_handler(Some(
                    move |evt: tray_icon::TrayIconEvent| {
                        let (name, data) = match &evt {
                            tray_icon::TrayIconEvent::Click {
                                button: tray_icon::MouseButton::Left,
                                ..
                            } => ("tray:click", Value::Null),
                            tray_icon::TrayIconEvent::Click {
                                button: tray_icon::MouseButton::Right,
                                ..
                            } => ("tray:right-click", Value::Null),
                            tray_icon::TrayIconEvent::DoubleClick { .. } => {
                                ("tray:double-click", Value::Null)
                            },
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
                        let _ = proxy
                            .send_event(UserEvent::OsEventScript(ipc::eval_os_event(name, &data)));
                    },
                ));
                Some(tray)
            },
            Err(e) => {
                crate::tynd_log!("System tray failed: {e}");
                None
            },
        });

    let call_tx_ipc = call_tx.clone();
    let proxy_for_ipc = proxy.clone();

    // Store WebView2 / WKWebView data in the OS app-data dir, not next to the exe.
    let default_app_name: String = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".to_string());
    let app_name = config.window.title.as_deref().unwrap_or(&default_app_name);
    let mut web_context = wry::WebContext::new(webview_data_dir(app_name));

    let mut wb = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_initialization_script(inject_window_label(PRIMARY_LABEL))
        .with_initialization_script(ipc::JS_PAGE_READY)
        .with_initialization_script(ipc::JS_SHIM)
        .with_ipc_handler(move |req: Request<String>| {
            dispatch::handle_ipc_body(req.into_body(), PRIMARY_LABEL, &call_tx_ipc, &proxy_for_ipc);
        })
        .with_drag_drop_handler(drag_drop_handler(PRIMARY_LABEL))
        .with_navigation_handler(navigation_handler(PRIMARY_LABEL))
        .with_on_page_load_handler(page_load_handler(PRIMARY_LABEL));

    if let Some(ref ua) = config.window.user_agent {
        wb = wb.with_user_agent(ua);
    }

    if debug {
        wb = wb.with_devtools(true);
        wb = wb.with_initialization_script(ipc::JS_DEV_FLAG);
    }

    // Binary IPC is needed in both dev and prod so `fs.readBinary` etc.
    // keep working against the dev server. Async so disk IO and multi-MB
    // compression run on the call pool instead of the UI thread.
    wb = wb.with_asynchronous_custom_protocol(
        "tynd-bin".into(),
        |_id, req: Request<Vec<u8>>, responder| scheme_bin::handle_async(req, responder),
    );

    if let Some(ref url) = config.dev_url {
        wb = wb.with_url(url);
    } else if let Some(ref dir) = config.frontend_dir {
        let dir = dir.clone();
        wb = wb
            .with_custom_protocol("tynd".into(), move |_id, req: Request<Vec<u8>>| {
                scheme::handle(&dir, &req)
            })
            .with_url("tynd://localhost/");
    } else {
        wb = wb.with_html(window::placeholder_html(app_name));
    }

    let webview = wb
        .build(&native_window)
        .expect("WebViewBuilder::build failed");

    if config.window.center.unwrap_or(false) {
        window::center_window(&native_window);
    }

    let exit_started = std::sync::Arc::new(AtomicBool::new(false));
    // tao has no dedicated minimize/maximize/fullscreen events — poll flags on
    // each Resized (triggered by every transition on every platform) and emit
    // synthetic events only when they actually flip.
    let mut last_state = WindowState::capture(&native_window);
    let mut secondaries: HashMap<String, SecondaryEntry> = HashMap::new();
    let mut window_id_to_label: HashMap<WindowId, String> = HashMap::new();
    window_id_to_label.insert(native_window.id(), PRIMARY_LABEL.into());
    let primary_id = native_window.id();
    let dev_url_owned = config.dev_url.clone();
    let frontend_dir_owned = config.frontend_dir.clone();

    // Per-window yield buffer — flushed in batches so a 10k-yield burst ends
    // up as ~30 `evaluate_script` calls instead of 10k. See YIELD_BATCH_MAX.
    let mut yield_buffer: HashMap<String, Vec<(String, Value)>> = HashMap::new();
    // Last zoom level passed to `setZoom`, keyed by window label. wry has no
    // getter so we cache the write.
    let mut zoom_levels: HashMap<String, f64> = HashMap::new();
    // Auto-hide flags set via CreateWindowOptions. `hideOnFocusLost` hides the
    // window when it blurs; `hideOnEscape` hides it on Escape keydown.
    let mut hide_on_focus_lost: HashMap<String, bool> = HashMap::new();
    let mut hide_on_escape: HashMap<String, bool> = HashMap::new();
    let flush_scheduled = std::sync::Arc::new(AtomicBool::new(false));

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::PageReady) => {
                native_window.set_visible(true);
                let _ = call_tx.send(BackendCall::lifecycle("on_ready"));
                for url in &initial_open_urls {
                    os::events::emit("app:open-url", &json!({ "url": url }));
                }
            },

            Event::UserEvent(UserEvent::FocusPrimary) => {
                native_window.set_minimized(false);
                native_window.set_visible(true);
                native_window.set_focus();
            },

            Event::UserEvent(UserEvent::ForceExit)
                if !exit_started.swap(true, Ordering::SeqCst) =>
            {
                crate::cleanup::run();
                std::process::exit(0);
            },

            Event::UserEvent(UserEvent::FlushYields) => {
                flush_scheduled.store(false, Ordering::SeqCst);
                flush_yields(&mut yield_buffer, &webview, &secondaries);
            },

            Event::UserEvent(UserEvent::Backend(evt)) => match evt {
                BackendEvent::Return { id, ok, value } => {
                    if id == "__tynd_on_close__" {
                        if !exit_started.swap(true, Ordering::SeqCst) {
                            crate::cleanup::run();
                            std::process::exit(0);
                        }
                        return;
                    }
                    if id.starts_with("__tynd_") {
                        return;
                    }
                    // Flush any pending yields for this stream before the
                    // terminating Return — otherwise the frontend iterator
                    // would finalize before the last chunks arrived.
                    flush_yields(&mut yield_buffer, &webview, &secondaries);
                    let label = dispatch::call_labels()
                        .remove(&id)
                        .map_or_else(|| PRIMARY_LABEL.into(), |(_, l)| l);
                    let wv = webview_for(&label, &webview, &secondaries);
                    let _ = wv.evaluate_script(&ipc::eval_resolve(&id, ok, &value));
                },
                BackendEvent::Yield { id, value } => {
                    let label = dispatch::call_labels()
                        .get(&id)
                        .map_or_else(|| PRIMARY_LABEL.into(), |e| e.clone());
                    let bucket = yield_buffer.entry(label).or_default();
                    bucket.push((id, value));
                    // Synchronous flush when any bucket gets hot — avoids
                    // memory growth if a single stream fires in a tight loop.
                    if bucket.len() >= YIELD_BATCH_MAX {
                        flush_yields(&mut yield_buffer, &webview, &secondaries);
                    } else if !flush_scheduled.swap(true, Ordering::SeqCst) {
                        let proxy = proxy.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(YIELD_FLUSH_MS));
                            let _ = proxy.send_event(UserEvent::FlushYields);
                        });
                    }
                },
                BackendEvent::Emit { name, payload, to } => {
                    // Reserved event: backend asked us to evaluate raw JS on
                    // the target webview(s). Used by `evalInFrontend` to
                    // inject code without round-tripping through an RPC call.
                    if name == "__tynd:eval__" {
                        if let Some(script) = payload.get("script").and_then(Value::as_str) {
                            if let Some(label) = to {
                                let wv = webview_for(&label, &webview, &secondaries);
                                let _ = wv.evaluate_script(script);
                            } else {
                                let _ = webview.evaluate_script(script);
                                for entry in secondaries.values() {
                                    let _ = entry.webview.evaluate_script(script);
                                }
                            }
                        }
                        return;
                    }
                    let script = ipc::eval_dispatch(&name, &payload);
                    if let Some(label) = to {
                        let wv = webview_for(&label, &webview, &secondaries);
                        let _ = wv.evaluate_script(&script);
                    } else {
                        let _ = webview.evaluate_script(&script);
                        for entry in secondaries.values() {
                            let _ = entry.webview.evaluate_script(&script);
                        }
                    }
                },
                BackendEvent::Reload => {
                    let _ = webview.evaluate_script(&ipc::eval_hide_error_overlay());
                    let _ = webview.evaluate_script("window.location.reload();");
                },
                BackendEvent::Error { message } => {
                    let _ = webview.evaluate_script(&ipc::eval_show_error_overlay(&message));
                },
            },

            Event::UserEvent(UserEvent::OsResult {
                label,
                id,
                ok,
                value,
            }) => {
                let script = ipc::eval_os_result(&id, ok, &value);
                let _ = webview_for(&label, &webview, &secondaries).evaluate_script(&script);
            },

            Event::UserEvent(UserEvent::WindowCmd {
                label,
                id,
                method,
                args,
            }) => {
                let (ok, value) = match method.as_str() {
                    "create" => match create_secondary(
                        target,
                        &args,
                        debug,
                        &call_tx,
                        &proxy,
                        dev_url_owned.as_deref(),
                        frontend_dir_owned.as_deref(),
                        &native_window,
                        &secondaries,
                    ) {
                        Ok((new_label, entry)) => {
                            window_id_to_label.insert(entry.window.id(), new_label.clone());
                            if args
                                .get("hideOnFocusLost")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                hide_on_focus_lost.insert(new_label.clone(), true);
                            }
                            if args
                                .get("hideOnEscape")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                            {
                                hide_on_escape.insert(new_label.clone(), true);
                            }
                            secondaries.insert(new_label, entry);
                            (true, Value::Null)
                        },
                        Err(e) => (false, Value::String(e)),
                    },
                    "close" => {
                        let target_label = args
                            .get("label")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if target_label == PRIMARY_LABEL {
                            (
                                false,
                                Value::String("cannot close the primary window".into()),
                            )
                        } else if let Some(entry) = secondaries.remove(&target_label) {
                            window_id_to_label.remove(&entry.window.id());
                            drop(entry);
                            (true, Value::Null)
                        } else {
                            (
                                false,
                                Value::String(format!("window '{target_label}' not found")),
                            )
                        }
                    },
                    "all" => {
                        let mut labels: Vec<Value> = vec![Value::String(PRIMARY_LABEL.into())];
                        labels.extend(secondaries.keys().cloned().map(Value::String));
                        (true, Value::Array(labels))
                    },
                    // WebView-level ops (reload, zoom, devtools) need access to
                    // the WebView handle which lives next to each Window entry.
                    "reload" => {
                        let wv = webview_for(&label, &webview, &secondaries);
                        let _ = wv.evaluate_script("window.location.reload();");
                        (true, Value::Null)
                    },
                    "setZoom" => {
                        let z = args.get("level").and_then(Value::as_f64).unwrap_or(1.0);
                        let wv = webview_for(&label, &webview, &secondaries);
                        match wv.zoom(z) {
                            Ok(()) => {
                                zoom_levels.insert(label.clone(), z);
                                (true, Value::Null)
                            },
                            Err(e) => (false, Value::String(format!("setZoom: {e}"))),
                        }
                    },
                    "getZoom" => (
                        true,
                        Value::from(zoom_levels.get(&label).copied().unwrap_or(1.0)),
                    ),
                    "openDevTools" => {
                        let wv = webview_for(&label, &webview, &secondaries);
                        #[cfg(debug_assertions)]
                        {
                            wv.open_devtools();
                            (true, Value::Null)
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            let _ = wv;
                            (
                                false,
                                Value::String(
                                    "openDevTools: only available in debug builds".into(),
                                ),
                            )
                        }
                    },
                    "closeDevTools" => {
                        let wv = webview_for(&label, &webview, &secondaries);
                        #[cfg(debug_assertions)]
                        {
                            wv.close_devtools();
                            (true, Value::Null)
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            let _ = wv;
                            (true, Value::Null)
                        }
                    },
                    "print" => {
                        let wv = webview_for(&label, &webview, &secondaries);
                        match wv.print() {
                            Ok(()) => (true, Value::Null),
                            Err(e) => (false, Value::String(format!("print: {e}"))),
                        }
                    },
                    "navigate" => {
                        let url = args.get("url").and_then(Value::as_str).unwrap_or("");
                        if url.is_empty() {
                            (false, Value::String("navigate: missing 'url'".into()))
                        } else {
                            let wv = webview_for(&label, &webview, &secondaries);
                            match wv.load_url(url) {
                                Ok(()) => (true, Value::Null),
                                Err(e) => (false, Value::String(format!("navigate: {e}"))),
                            }
                        }
                    },
                    "loadHtml" => {
                        let html = args.get("html").and_then(Value::as_str).unwrap_or("");
                        let wv = webview_for(&label, &webview, &secondaries);
                        match wv.load_html(html) {
                            Ok(()) => (true, Value::Null),
                            Err(e) => (false, Value::String(format!("loadHtml: {e}"))),
                        }
                    },
                    "getUrl" => {
                        let wv = webview_for(&label, &webview, &secondaries);
                        match wv.url() {
                            Ok(u) => (true, Value::String(u)),
                            Err(e) => (false, Value::String(format!("getUrl: {e}"))),
                        }
                    },
                    "showContextMenu" => {
                        let win: Option<&Window> = if label == PRIMARY_LABEL {
                            Some(&native_window)
                        } else {
                            secondaries.get(&label).map(|e| &e.window)
                        };
                        match win {
                            Some(w) => match menu::show_context_menu(w, &args) {
                                Ok(()) => (true, Value::Null),
                                Err(e) => (false, Value::String(e)),
                            },
                            None => (false, Value::String(format!("window '{label}' not found"))),
                        }
                    },
                    _ => {
                        let win: Option<&Window> = if label == PRIMARY_LABEL {
                            Some(&native_window)
                        } else {
                            secondaries.get(&label).map(|e| &e.window)
                        };
                        match win {
                            Some(w) => match os::window_cmd::dispatch(w, &method, &args) {
                                Ok(v) => (true, v),
                                Err(e) => (false, Value::String(e)),
                            },
                            None => (false, Value::String(format!("window '{label}' not found"))),
                        }
                    },
                };
                let script = ipc::eval_os_result(&id, ok, &value);
                let _ = webview_for(&label, &webview, &secondaries).evaluate_script(&script);
            },

            Event::UserEvent(UserEvent::TrayCmd {
                label,
                id,
                method,
                args,
            }) => {
                let (ok, value) = match system_tray.as_ref() {
                    Some(t) => match tray::dispatch(t, &method, &args) {
                        Ok(v) => (true, v),
                        Err(e) => (false, Value::String(e)),
                    },
                    None => (
                        false,
                        Value::String(
                            "tray: no tray configured — set `tray` in tynd.config.ts first".into(),
                        ),
                    ),
                };
                let script = ipc::eval_os_result(&id, ok, &value);
                let _ = webview_for(&label, &webview, &secondaries).evaluate_script(&script);
            },

            Event::UserEvent(UserEvent::OsEventScript(script)) => {
                let _ = webview.evaluate_script(&script);
                for entry in secondaries.values() {
                    let _ = entry.webview.evaluate_script(&script);
                }
            },

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                window_id,
                ..
            } => {
                let label = label_for(window_id, &window_id_to_label);
                if window_id != primary_id {
                    // Secondary window: close immediately, no cancel window.
                    cancel_streams_for_label(&label, &call_tx);
                    os::events::emit("window:closed", &json!({ "label": label }));
                    if let Some(entry) = secondaries.remove(&label) {
                        window_id_to_label.remove(&entry.window.id());
                        drop(entry);
                    }
                    // `quitOnLastWindowClosed`: when the primary is already
                    // hidden and no secondaries remain, the app has nothing
                    // left to show. Exit cleanly.
                    if config.quit_on_last_window_closed
                        && secondaries.is_empty()
                        && !native_window.is_visible()
                        && !exit_started.swap(true, Ordering::SeqCst)
                    {
                        crate::cleanup::run();
                        std::process::exit(0);
                    }
                    return;
                }
                // Primary window: frontend/backend get 500ms to call
                // `tyndWindow.cancelClose()` before we hide + exit. The existing
                // 2s watchdog still covers backend handler timeouts.
                os::window_cmd::reset_close_cancel();
                os::events::emit("window:close-requested", &json!({ "label": label }));

                let proxy_cancel = proxy.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    if os::window_cmd::close_cancelled() {
                        return;
                    }
                    let _ = proxy_cancel.send_event(UserEvent::ProceedClose);
                });
            },

            Event::UserEvent(UserEvent::ProceedClose) => {
                if os::window_cmd::close_cancelled() {
                    return;
                }
                native_window.set_visible(false);
                let _ = call_tx.send(BackendCall::lifecycle("on_close"));
                // Watchdog: if the backend handler hangs, force-exit after 2s.
                let proxy = proxy.clone();
                let exit_started = exit_started.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if !exit_started.load(Ordering::SeqCst) {
                        let _ = proxy.send_event(UserEvent::ForceExit);
                    }
                });
            },

            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                window_id,
                ..
            } => {
                let label = label_for(window_id, &window_id_to_label);
                os::events::emit(
                    "window:resized",
                    &json!({ "label": &label, "width": size.width, "height": size.height }),
                );
                if window_id == primary_id {
                    last_state.diff_and_emit(&native_window, &label);
                } else if let Some(entry) = secondaries.get_mut(&label) {
                    // Disjoint-field borrow: state mutated while window read-only.
                    let SecondaryEntry { window, state, .. } = entry;
                    state.diff_and_emit(window, &label);
                }
            },

            Event::WindowEvent {
                event: WindowEvent::Moved(pos),
                window_id,
                ..
            } => {
                let label = label_for(window_id, &window_id_to_label);
                os::events::emit(
                    "window:moved",
                    &json!({ "label": label, "x": pos.x, "y": pos.y }),
                );
            },

            Event::WindowEvent {
                event: WindowEvent::Focused(focused),
                window_id,
                ..
            } => {
                let label = label_for(window_id, &window_id_to_label);
                os::events::emit(
                    if focused {
                        "window:focused"
                    } else {
                        "window:blurred"
                    },
                    &json!({ "label": label }),
                );
                if !focused && hide_on_focus_lost.get(&label).copied().unwrap_or(false) {
                    if let Some(entry) = secondaries.get(&label) {
                        entry.window.set_visible(false);
                    }
                }
            },

            Event::WindowEvent {
                event:
                    WindowEvent::KeyboardInput {
                        event: key_event, ..
                    },
                window_id,
                ..
            } if key_event.physical_key == tao::keyboard::KeyCode::Escape
                && key_event.state == tao::event::ElementState::Pressed =>
            {
                let label = label_for(window_id, &window_id_to_label);
                if hide_on_escape.get(&label).copied().unwrap_or(false) {
                    if let Some(entry) = secondaries.get(&label) {
                        entry.window.set_visible(false);
                    }
                }
            },

            Event::WindowEvent {
                event: WindowEvent::ThemeChanged(theme),
                window_id,
                ..
            } => {
                let label = label_for(window_id, &window_id_to_label);
                let name = if matches!(theme, Theme::Dark) {
                    "dark"
                } else {
                    "light"
                };
                os::events::emit(
                    "window:theme-changed",
                    &json!({ "label": label, "theme": name }),
                );
            },

            Event::WindowEvent {
                event: WindowEvent::ScaleFactorChanged { scale_factor, .. },
                window_id,
                ..
            } => {
                let label = label_for(window_id, &window_id_to_label);
                os::events::emit(
                    "window:dpi-changed",
                    &json!({ "label": label, "scale": scale_factor }),
                );
            },

            _ => {},
        }
    })
}

/// Cheap deep-link detector: `<scheme>://...` where scheme begins with a
/// letter and contains only RFC-3986 scheme characters. We don't restrict
/// to configured protocols — app authors filter by scheme in their handler
/// if they want to narrow.
fn looks_like_url(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once("://") else {
        return false;
    };
    if scheme.is_empty() || rest.is_empty() {
        return false;
    }
    let mut chars = scheme.chars();
    let first = chars.next();
    if !first.is_some_and(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// Cancel every active streaming call whose origin window just closed. Looks
/// up ids in `dispatch::call_labels()` and sends `BackendCall::Cancel` for
/// each match. Without this, a closed secondary window leaves its async
/// generators running forever on the backend.
fn cancel_streams_for_label(label: &str, call_tx: &std::sync::mpsc::Sender<BackendCall>) {
    let labels = dispatch::call_labels();
    let orphaned: Vec<String> = labels
        .iter()
        .filter_map(|e| (e.value() == label).then(|| e.key().clone()))
        .collect();
    for id in orphaned {
        labels.remove(&id);
        let _ = call_tx.send(BackendCall::Cancel { id });
    }
}

/// Drain the yield buffer into one `evaluate_script` call per webview. Keeps
/// per-stream FIFO order (each id's yields arrive together via
/// `__tynd_yield_batch__` on the frontend shim).
fn flush_yields(
    buffer: &mut HashMap<String, Vec<(String, Value)>>,
    primary: &WebView,
    secondaries: &HashMap<String, SecondaryEntry>,
) {
    for (label, pairs) in buffer.drain() {
        if pairs.is_empty() {
            continue;
        }
        let wv = webview_for(&label, primary, secondaries);
        let _ = wv.evaluate_script(&ipc::eval_yield_batch(&pairs));
    }
}

fn webview_for<'a>(
    label: &str,
    primary: &'a WebView,
    secondaries: &'a HashMap<String, SecondaryEntry>,
) -> &'a WebView {
    if label == PRIMARY_LABEL {
        primary
    } else {
        secondaries.get(label).map_or(primary, |e| &e.webview)
    }
}

#[cfg(target_os = "windows")]
fn with_parent_window(wb: WindowBuilder, parent: &Window) -> WindowBuilder {
    use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
    wb.with_parent_window(parent.hwnd() as _)
}

#[cfg(target_os = "macos")]
fn with_parent_window(wb: WindowBuilder, parent: &Window) -> WindowBuilder {
    use tao::platform::macos::{WindowBuilderExtMacOS, WindowExtMacOS};
    wb.with_parent_window(parent.ns_window())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn with_parent_window(wb: WindowBuilder, _parent: &Window) -> WindowBuilder {
    // GTK's transient-for pattern isn't exposed by tao — modalTo quietly
    // degrades to a regular always-on-top window. Apps that need strict
    // modal semantics on Linux should set `alwaysOnTop: true` + block
    // parent input via `setEnabled(false)` in their own handler.
    wb
}

fn label_for(id: WindowId, map: &HashMap<WindowId, String>) -> String {
    map.get(&id)
        .cloned()
        .unwrap_or_else(|| PRIMARY_LABEL.into())
}

/// Build a DragDrop handler bound to a window label. Emits `window:drag-enter`,
/// `window:drag-over`, `window:drag-leave`, `window:drop` with paths + cursor
/// position. Returns `false` so the WebView still runs default HTML5 DnD.
fn drag_drop_handler(label: &str) -> impl Fn(DragDropEvent) -> bool + 'static {
    let label = label.to_string();
    move |evt| {
        match evt {
            DragDropEvent::Enter { paths, position } => {
                let paths: Vec<String> = paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                os::events::emit(
                    "window:drag-enter",
                    &json!({ "label": label, "paths": paths, "x": position.0, "y": position.1 }),
                );
            },
            DragDropEvent::Over { position } => {
                os::events::emit(
                    "window:drag-over",
                    &json!({ "label": label, "x": position.0, "y": position.1 }),
                );
            },
            DragDropEvent::Drop { paths, position } => {
                let paths: Vec<String> = paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                os::events::emit(
                    "window:drop",
                    &json!({ "label": label, "paths": paths, "x": position.0, "y": position.1 }),
                );
            },
            DragDropEvent::Leave => {
                os::events::emit("window:drag-leave", &json!({ "label": label }));
            },
            _ => {},
        }
        false
    }
}

/// Navigation hook — emits `webview:navigation` and blocks the load when the
/// security policy explicitly denies the URL. Empty / unconfigured policy
/// lets everything through.
fn navigation_handler(label: &str) -> impl Fn(String) -> bool + 'static {
    let label = label.to_string();
    move |url: String| {
        let allowed = os::security::check_http(&url).is_ok();
        os::events::emit(
            "webview:navigation",
            &json!({ "label": label, "url": url, "allowed": allowed }),
        );
        allowed
    }
}

/// Page-load observer — fires once when the document starts loading and
/// once when it finishes. Frontend listens via `tyndWindow.onPageLoad`.
fn page_load_handler(label: &str) -> impl Fn(PageLoadEvent, String) + 'static {
    let label = label.to_string();
    move |event, url| {
        let phase = match event {
            PageLoadEvent::Started => "started",
            PageLoadEvent::Finished => "finished",
        };
        os::events::emit(
            "webview:page-load",
            &json!({ "label": label, "phase": phase, "url": url }),
        );
    }
}

/// Init script injected into every webview so frontend code knows which window
/// it's running in (for filtering broadcast `window:*` events by label).
fn inject_window_label(label: &str) -> String {
    // Escape any quote / backslash so the label can't break the script.
    let esc = label.replace('\\', "\\\\").replace('"', "\\\"");
    format!("window.__TYND_WINDOW_LABEL__ = \"{esc}\";")
}

/// Create a secondary window + webview + state tracker. Called on the main
/// thread inside the event loop (wry/tao require it).
#[allow(clippy::too_many_arguments)]
fn create_secondary(
    target: &EventLoopWindowTarget<UserEvent>,
    args: &Value,
    debug: bool,
    call_tx: &std::sync::mpsc::Sender<BackendCall>,
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    dev_url: Option<&str>,
    frontend_dir: Option<&str>,
    primary: &Window,
    secondaries: &HashMap<String, SecondaryEntry>,
) -> Result<(String, SecondaryEntry), String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if label.is_empty() {
        return Err("create: missing 'label'".into());
    }
    if label == PRIMARY_LABEL {
        return Err(format!("create: label '{PRIMARY_LABEL}' is reserved"));
    }

    let title = args
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(&label)
        .to_string();
    let width = args.get("width").and_then(Value::as_f64).unwrap_or(800.0);
    let height = args.get("height").and_then(Value::as_f64).unwrap_or(600.0);

    let mut wb = WindowBuilder::new()
        .with_title(&title)
        .with_inner_size(LogicalSize::new(width, height))
        .with_resizable(
            args.get("resizable")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )
        .with_decorations(
            args.get("decorations")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )
        .with_visible(true);
    if args
        .get("alwaysOnTop")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        wb = wb.with_always_on_top(true);
    }
    // Modal / owned child window: look up the parent by label and pass the
    // native handle to tao's platform builder. Linux has no equivalent, so
    // modalTo is a no-op there.
    if let Some(parent_label) = args.get("modalTo").and_then(Value::as_str) {
        let parent: &Window = if parent_label == PRIMARY_LABEL {
            primary
        } else {
            secondaries
                .get(parent_label)
                .map(|e| &e.window)
                .ok_or_else(|| format!("create: parent window '{parent_label}' not found"))?
        };
        wb = with_parent_window(wb, parent);
    }
    let window = wb.build(target).map_err(|e| format!("build window: {e}"))?;

    let call_tx = call_tx.clone();
    let proxy = proxy.clone();
    let label_for_ipc = label.clone();

    let mut wvb = WebViewBuilder::new()
        .with_initialization_script(inject_window_label(&label))
        // Parity with the primary window: page-ready fires DOMContentLoaded,
        // keeping secondary windows on the same visibility + lifecycle path
        // if future refactors gate any behaviour on it.
        .with_initialization_script(ipc::JS_PAGE_READY)
        .with_initialization_script(ipc::JS_SHIM)
        .with_ipc_handler(move |req: Request<String>| {
            dispatch::handle_ipc_body(req.into_body(), &label_for_ipc, &call_tx, &proxy);
        })
        .with_drag_drop_handler(drag_drop_handler(&label))
        .with_navigation_handler(navigation_handler(&label))
        .with_on_page_load_handler(page_load_handler(&label));

    if let Some(ua) = args.get("userAgent").and_then(Value::as_str) {
        wvb = wvb.with_user_agent(ua);
    }
    if debug {
        wvb = wvb.with_devtools(true);
        wvb = wvb.with_initialization_script(ipc::JS_DEV_FLAG);
    }
    wvb = wvb.with_asynchronous_custom_protocol(
        "tynd-bin".into(),
        |_id, req: Request<Vec<u8>>, responder| scheme_bin::handle_async(req, responder),
    );

    // `url` option overrides the default frontend mapping — lets apps point
    // secondary windows at /settings, /about, external docs, etc.
    let custom_url = args.get("url").and_then(Value::as_str);
    if let Some(url) = custom_url {
        wvb = wvb.with_url(url);
    } else if let Some(url) = dev_url {
        wvb = wvb.with_url(url);
    } else if let Some(dir) = frontend_dir {
        let dir = dir.to_string();
        wvb = wvb
            .with_custom_protocol("tynd".into(), move |_id, req: Request<Vec<u8>>| {
                scheme::handle(&dir, &req)
            })
            .with_url("tynd://localhost/");
    }

    let webview = wvb
        .build(&window)
        .map_err(|e| format!("build webview: {e}"))?;
    let state = WindowState::capture(&window);
    Ok((
        label,
        SecondaryEntry {
            window,
            webview,
            state,
        },
    ))
}

/// Last-seen minimize/maximize/fullscreen flags. Used to emit synthetic
/// transition events since tao doesn't surface them directly.
struct WindowState {
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
}

impl WindowState {
    fn capture(win: &Window) -> Self {
        Self {
            minimized: win.is_minimized(),
            maximized: win.is_maximized(),
            fullscreen: win.fullscreen().is_some(),
        }
    }

    /// Compare current window flags to the last seen; emit events on flips.
    fn diff_and_emit(&mut self, win: &Window, label: &str) {
        let next = Self::capture(win);
        if next.minimized != self.minimized {
            os::events::emit(
                if next.minimized {
                    "window:minimized"
                } else {
                    "window:unminimized"
                },
                &json!({ "label": label }),
            );
        }
        if next.maximized != self.maximized {
            os::events::emit(
                if next.maximized {
                    "window:maximized"
                } else {
                    "window:unmaximized"
                },
                &json!({ "label": label }),
            );
        }
        if next.fullscreen != self.fullscreen {
            os::events::emit(
                if next.fullscreen {
                    "window:fullscreen"
                } else {
                    "window:unfullscreen"
                },
                &json!({ "label": label }),
            );
        }
        *self = next;
    }
}
