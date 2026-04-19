use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget},
    window::{Theme, Window, WindowBuilder, WindowId},
};
use wry::{http::Request, WebView, WebViewBuilder};

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
    /// Sent by a timeout thread when on_close takes too long
    ForceExit,
    /// Sent 500ms after CloseRequested if `cancelClose()` wasn't called.
    ProceedClose,
}

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

    let _system_tray: Option<tray_icon::TrayIcon> =
        config.tray.as_ref().and_then(|tc| match tray::build(tc) {
            Ok(tray) => {
                let proxy = proxy.clone();
                tray_icon::TrayIconEvent::set_event_handler(Some(
                    move |evt: tray_icon::TrayIconEvent| {
                        let name = match &evt {
                            tray_icon::TrayIconEvent::Click {
                                button: tray_icon::MouseButton::Left,
                                ..
                            } => "tray:click",
                            tray_icon::TrayIconEvent::Click {
                                button: tray_icon::MouseButton::Right,
                                ..
                            } => "tray:right-click",
                            tray_icon::TrayIconEvent::DoubleClick { .. } => "tray:double-click",
                            _ => return,
                        };
                        let _ = proxy.send_event(UserEvent::OsEventScript(ipc::eval_os_event(
                            name,
                            &Value::Null,
                        )));
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
        });

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

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(UserEvent::PageReady) => {
                native_window.set_visible(true);
                let _ = call_tx.send(BackendCall::lifecycle("on_ready"));
            },

            Event::UserEvent(UserEvent::ForceExit)
                if !exit_started.swap(true, Ordering::SeqCst) =>
            {
                crate::cleanup::run();
                std::process::exit(0);
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
                    let _ = webview.evaluate_script(&ipc::eval_resolve(&id, ok, &value));
                },
                BackendEvent::Yield { id, value } => {
                    let _ = webview.evaluate_script(&ipc::eval_yield(&id, &value));
                },
                BackendEvent::Emit { name, payload } => {
                    let _ = webview.evaluate_script(&ipc::eval_dispatch(&name, &payload));
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
                    ) {
                        Ok((new_label, entry)) => {
                            window_id_to_label.insert(entry.window.id(), new_label.clone());
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
                    os::events::emit("window:closed", &json!({ "label": label }));
                    if let Some(entry) = secondaries.remove(&label) {
                        window_id_to_label.remove(&entry.window.id());
                        drop(entry);
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

fn label_for(id: WindowId, map: &HashMap<WindowId, String>) -> String {
    map.get(&id)
        .cloned()
        .unwrap_or_else(|| PRIMARY_LABEL.into())
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
    let window = wb.build(target).map_err(|e| format!("build window: {e}"))?;

    let call_tx = call_tx.clone();
    let proxy = proxy.clone();
    let label_for_ipc = label.clone();

    let mut wvb = WebViewBuilder::new()
        .with_initialization_script(inject_window_label(&label))
        .with_initialization_script(ipc::JS_SHIM)
        .with_ipc_handler(move |req: Request<String>| {
            dispatch::handle_ipc_body(req.into_body(), &label_for_ipc, &call_tx, &proxy);
        });
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
