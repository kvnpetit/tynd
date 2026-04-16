use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
};
use wry::{http::Request, WebViewBuilder};

use crate::{
    ipc, os,
    runtime::{BackendBridge, BackendCall, BackendEvent, MenuItemDef, TrayConfig},
    scheme, window,
};

#[derive(Debug)]
enum UserEvent {
    Backend(BackendEvent),
    PageReady,
    OsResult {
        id: String,
        ok: bool,
        value: Value,
    },
    OsEvent {
        name: String,
        data: Value,
    },
    WindowCmd {
        id: String,
        method: String,
        args: Value,
    },
    /// Sent by a timeout thread when on_close takes too long
    ForceExit,
}

pub fn run_app(bridge: BackendBridge, debug: bool) -> ! {
    let BackendBridge {
        config,
        call_tx,
        event_rx,
    } = bridge;

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Relay BackendEvents → tao UserEvents
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
            let _ = proxy.send_event(UserEvent::OsEvent {
                name: "menu:action".into(),
                data,
            });
        }));
    }

    let _app_menu: Option<muda::Menu> = if !config.menu.is_empty() {
        match build_muda_bar(&config.menu) {
            Ok(menu) => {
                init_menu_bar(&menu, &native_window);
                Some(menu)
            },
            Err(e) => {
                crate::vorn_log!("Menu build failed: {e}");
                None
            },
        }
    } else {
        None
    };

    let _system_tray: Option<tray_icon::TrayIcon> =
        config.tray.as_ref().and_then(|tc| match build_tray(tc) {
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
                        let _ = proxy.send_event(UserEvent::OsEvent {
                            name: name.into(),
                            data: Value::Null,
                        });
                    },
                ));
                Some(tray)
            },
            Err(e) => {
                crate::vorn_log!("System tray failed: {e}");
                None
            },
        });

    let call_tx_ipc = call_tx.clone();
    let proxy_for_ipc = proxy.clone();

    // Store WebView2 / WKWebView data in the OS app-data dir, not next to the exe.
    let default_app_name: String = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "vorn".to_string());
    let app_name = config.window.title.as_deref().unwrap_or(&default_app_name);
    let mut web_context = wry::WebContext::new(webview_data_dir(app_name));

    let mut wb = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_initialization_script(ipc::JS_PAGE_READY)
        .with_initialization_script(ipc::JS_SHIM)
        .with_ipc_handler(move |req: Request<String>| {
            let body = req.body().trim();
            if body.is_empty() {
                return;
            }

            // Fast path: the IPC shim always produces {"type":"call",...} via
            // JSON.stringify (compact, no spaces). Detect without a full parse
            // and forward directly to Bun, skipping one parse+serialize cycle.
            if body.starts_with(r#"{"type":"call""#) {
                let _ = call_tx_ipc.send(BackendCall::Raw(body.to_owned()));
                return;
            }

            let Ok(v) = serde_json::from_str::<Value>(body) else {
                return;
            };

            // Page-ready signal (fired once on DOMContentLoaded)
            if v.get("__vorn_page_ready")
                .and_then(|f| f.as_bool())
                .unwrap_or(false)
            {
                let _ = proxy_for_ipc.send_event(UserEvent::PageReady);
                return;
            }

            if let Some("os_call") = v.get("type").and_then(|t| t.as_str()) {
                let id = v["id"].as_str().unwrap_or("").to_string();
                let api = v["api"].as_str().unwrap_or("").to_string();
                let method = v["method"].as_str().unwrap_or("").to_string();
                let args = v["args"].clone();

                if api == "window" {
                    // Window commands must run on the main thread
                    let _ = proxy_for_ipc.send_event(UserEvent::WindowCmd { id, method, args });
                } else {
                    // Dedicated thread per OS call — NOT rayon, which is CPU-bound.
                    // Dialogs block for seconds (user interaction); blocking a rayon
                    // worker would starve the CPU thread pool.
                    let proxy = proxy_for_ipc.clone();
                    std::thread::spawn(move || {
                        let result = os::dispatch(&api, &method, &args);
                        let (ok, value) = match result {
                            Ok(v) => (true, v),
                            Err(e) => (false, Value::String(e)),
                        };
                        let _ = proxy.send_event(UserEvent::OsResult { id, ok, value });
                    });
                }
            }
        });

    if debug {
        wb = wb.with_devtools(true);
        wb = wb.with_initialization_script(ipc::JS_DEV_FLAG);
    }

    if let Some(ref url) = config.dev_url {
        wb = wb.with_url(url);
    } else if let Some(ref dir) = config.frontend_dir {
        let dir = dir.clone();
        wb = wb
            .with_custom_protocol("bv".into(), move |_id, req: Request<Vec<u8>>| {
                scheme::handle(&dir, req)
            })
            .with_url("bv://localhost/");
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
    event_loop.run(move |event, _, control_flow| {
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
                    if id == "__vorn_on_close__" {
                        if !exit_started.swap(true, Ordering::SeqCst) {
                            crate::cleanup::run();
                            std::process::exit(0);
                        }
                        return;
                    }
                    if id.starts_with("__vorn_") {
                        return;
                    }
                    let _ = webview.evaluate_script(&ipc::eval_resolve(&id, ok, &value));
                },
                BackendEvent::Emit { name, payload } => {
                    let _ = webview.evaluate_script(&ipc::eval_dispatch(&name, &payload));
                },
            },

            Event::UserEvent(UserEvent::OsResult { id, ok, value }) => {
                let _ = webview.evaluate_script(&ipc::eval_os_result(&id, ok, &value));
            },

            Event::UserEvent(UserEvent::WindowCmd { id, method, args }) => {
                let (ok, value) = match os::window_cmd::dispatch(&native_window, &method, &args) {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
                };
                let _ = webview.evaluate_script(&ipc::eval_os_result(&id, ok, &value));
            },

            Event::UserEvent(UserEvent::OsEvent { name, data }) => {
                let _ = webview.evaluate_script(&ipc::eval_os_event(&name, &data));
            },

            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                native_window.set_visible(false);
                let _ = call_tx.send(BackendCall::lifecycle("on_close"));
                // Fall back to ForceExit after 2 seconds in case the handler hangs.
                let proxy = proxy.clone();
                let exit_started = exit_started.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(2000));
                    if !exit_started.load(Ordering::SeqCst) {
                        let _ = proxy.send_event(UserEvent::ForceExit);
                    }
                });
            },

            _ => {},
        }
    })
}

/// Return an OS-appropriate directory for WebView persistent data.
/// Windows : %LOCALAPPDATA%\<app>\WebView2
/// macOS   : ~/Library/Application Support/<app>/WebView2
/// Linux   : ~/.local/share/<app>/webview
fn webview_data_dir(app_name: &str) -> Option<std::path::PathBuf> {
    // Sanitize app name: keep [A-Za-z0-9_], collapse other chars to single dashes.
    let mut safe = String::new();
    let mut prev_dash = true; // start true to suppress leading dashes
    for c in app_name.chars() {
        if c.is_alphanumeric() || c == '_' {
            safe.push(c);
            prev_dash = false;
        } else if !prev_dash {
            safe.push('-');
            prev_dash = true;
        }
    }
    while safe.ends_with('-') {
        safe.pop();
    }
    if safe.is_empty() {
        safe = "vorn".to_string();
    }
    dirs::data_local_dir().map(|d| d.join(safe).join("WebView2"))
}

fn build_muda_bar(items: &[MenuItemDef]) -> Result<muda::Menu, String> {
    let menu = muda::Menu::new();
    for item in items {
        let label = item.label.as_deref().unwrap_or("");
        let enabled = item.enabled.unwrap_or(true);
        let sub = muda::Submenu::new(label, enabled);
        fill_items(
            &|i: &dyn muda::IsMenuItem| sub.append(i),
            item.items.as_deref().unwrap_or(&[]),
        )?;
        menu.append(&sub).map_err(|e| e.to_string())?;
    }
    Ok(menu)
}

/// Recursively fill a menu container using a caller-provided append closure.
fn fill_items(
    append: &dyn Fn(&dyn muda::IsMenuItem) -> muda::Result<()>,
    items: &[MenuItemDef],
) -> Result<(), String> {
    for item in items {
        match item.kind.as_deref() {
            Some("separator") => {
                append(&muda::PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
            },
            Some("submenu") => {
                let label = item.label.as_deref().unwrap_or("");
                let enabled = item.enabled.unwrap_or(true);
                let sub = muda::Submenu::new(label, enabled);
                fill_items(
                    &|i: &dyn muda::IsMenuItem| sub.append(i),
                    item.items.as_deref().unwrap_or(&[]),
                )?;
                append(&sub).map_err(|e| e.to_string())?;
            },
            _ => {
                if let Some(role) = &item.role {
                    if let Some(pi) = role_to_predefined(role) {
                        append(&pi).map_err(|e| e.to_string())?;
                        continue;
                    }
                }
                if let Some(label) = &item.label {
                    let id = muda::MenuId::new(item.id.as_deref().unwrap_or(label));
                    let mi = muda::MenuItem::with_id(id, label, item.enabled.unwrap_or(true), None);
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
        }
    }
    Ok(())
}

fn role_to_predefined(role: &str) -> Option<muda::PredefinedMenuItem> {
    use muda::PredefinedMenuItem as P;
    Some(match role {
        "separator" => P::separator(),
        "quit" => P::quit(None),
        "copy" => P::copy(None),
        "cut" => P::cut(None),
        "paste" => P::paste(None),
        "undo" => P::undo(None),
        "redo" => P::redo(None),
        "selectAll" => P::select_all(None),
        "minimize" => P::minimize(None),
        "close" => P::close_window(None),
        "about" => P::about(None::<&str>, None),
        _ => return None,
    })
}

fn init_menu_bar(menu: &muda::Menu, window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe {
            if let Err(e) = menu.init_for_hwnd(window.hwnd()) {
                crate::vorn_log!("menu.init_for_hwnd: {e}");
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        menu.init_for_nsapp();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (menu, window);
    }
}

fn build_tray(cfg: &TrayConfig) -> Result<tray_icon::TrayIcon, String> {
    let icon = os::icon::load_tray(&cfg.icon)?;

    let tray_menu = muda::Menu::new();
    if let Some(items) = &cfg.menu {
        fill_items(&|i: &dyn muda::IsMenuItem| tray_menu.append(i), items)?;
    }

    let mut builder = tray_icon::TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(tray_menu));

    if let Some(tt) = &cfg.tooltip {
        builder = builder.with_tooltip(tt);
    }

    builder.build().map_err(|e| e.to_string())
}
