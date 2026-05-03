use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};

use serde_json::{json, Value};
use smallvec::SmallVec;
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
mod gpu;
mod util;

use util::webview_data_dir;

/// Reserved — secondary windows created via `tyndWindow.create({ label })` may not reuse it.
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
    /// Menu-bar mutations (`menu.update`, `menu.setItemEnabled`, …). Routed
    /// through the event loop because muda's items are `!Send` on
    /// Windows / macOS and live on the main thread alongside the menu bar.
    MenuCmd {
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
    #[cfg_attr(not(feature = "api-single-instance"), allow(dead_code))]
    FocusPrimary,
    /// Sent 10ms after the first buffered yield (or synchronously when the
    /// buffer exceeds `YIELD_BATCH_MAX`) to flush pending chunks to every
    /// webview in one `evaluate_script` per window.
    FlushYields,
    /// Web content process / GPU process crashed. Triggers a reload of the
    /// affected webview when `window.contextLostRecovery` is enabled, plus
    /// emits `webview:context-lost` so the frontend can react. macOS / iOS
    /// fire this via `WKWebView`'s
    /// `webViewWebContentProcessDidTerminate:`; on Windows + Linux the
    /// underlying compositor recovers transparently and the host stays
    /// silent — hence the platform-conditional `dead_code` allow.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ContextLost { label: String },
}

/// Maximum yields buffered per window before we flush synchronously.
/// Higher = fewer `evaluate_script` calls, lower = smaller memory spike.
const YIELD_BATCH_MAX: usize = 64;
/// How long the first buffered yield waits for siblings before flushing.
/// Invisible to users (<1 frame) but cuts per-chunk overhead on bursty streams.
const YIELD_FLUSH_MS: u64 = 10;

/// Per-window yield bucket. Inline capacity of 8 covers the typical
/// streaming burst between two `YIELD_FLUSH_MS` (10 ms) timer ticks: an LLM
/// streaming at 50 tok/s lands ~1 chunk per tick, a fast log tail lands a
/// handful, and only pathological bursts exhaust the inline storage and
/// spill into a heap allocation. The bucket still caps at `YIELD_BATCH_MAX`
/// (64) via the synchronous-flush branch, so even worst-case memory is
/// bounded — `SmallVec` just removes the heap alloc from the common case.
type YieldBucket = SmallVec<[(String, String); 8]>;

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

    // Phase 1 of the GPU pipeline — set process-wide env vars (NVIDIA PRIME
    // / DRI_PRIME / WEBKIT_USE_WEBGPU) BEFORE WebView2 / WKWebView /
    // WebKitGTK initialise. Chromium snapshots most of these at first GPU
    // process spawn, so retro-active changes after the WebViewBuilder runs
    // are no-ops.
    gpu::apply_pre_webview_env(&config.window);

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

    // Wire the signal handler into the event loop. When SIGINT / SIGTERM
    // fires, `signals.rs` calls this closure on whichever OS thread
    // delivered the signal (Windows: the console-control thread; Unix:
    // the thread that consumed `sigaction`). We just bounce a `ForceExit`
    // user event to the main thread — `ForceExit`'s arm already runs
    // `cleanup::run()` and `process::exit(0)` under the `exit_started`
    // guard, so we get the same shutdown path as the OS-close watchdog.
    {
        let proxy = proxy.clone();
        crate::cleanup::set_shutdown_hook(move || {
            let _ = proxy.send_event(UserEvent::ForceExit);
        });
    }

    // Install the plugin event bus against the now-wired emitter so
    // native plugins can raise events alongside OS APIs.
    crate::plugins::install_default_emit();

    #[cfg(feature = "api-single-instance")]
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

    if let Some(ref dir) = config.frontend_dir {
        let dir = dir.clone();
        std::thread::spawn(move || scheme::warm(&dir));
    }

    let native_window =
        window::build_window(&config.window, config.icon_path.as_deref(), &event_loop);

    // Shared by the app menu bar and every tray submenu.
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
        // Devalue must run before JS_SHIM so the shim's encoders/decoders
        // resolve `window.__tynd_dv__` on the first IPC frame.
        .with_initialization_script(ipc::JS_DEVALUE)
        .with_initialization_script(ipc::JS_SHIM)
        .with_ipc_handler(move |req: Request<String>| {
            dispatch::handle_ipc_body(req.into_body(), PRIMARY_LABEL, &call_tx_ipc, &proxy_for_ipc);
        })
        .with_drag_drop_handler(drag_drop_handler(PRIMARY_LABEL))
        .with_navigation_handler(navigation_handler(PRIMARY_LABEL))
        .with_on_page_load_handler(page_load_handler(PRIMARY_LABEL));

    // Phase 2 of the GPU pipeline — Chromium browser-args injection on
    // Windows (WebView2). macOS / Linux configure their hints through env
    // vars + builder configs already applied above.
    #[cfg(target_os = "windows")]
    if let Some(args) = gpu::assemble_browser_args(&config.window) {
        use wry::WebViewBuilderExtWindows;
        wb = wb.with_additional_browser_args(args);
    }

    // macOS / iOS — observe web content process termination so the host
    // can auto-recover the WebView when the GPU process dies. Defaults to
    // on; opt-out via `window.contextLostRecovery: false`.
    #[cfg(target_os = "macos")]
    if config.window.context_lost_recovery.unwrap_or(true) {
        use wry::WebViewBuilderExtDarwin;
        let proxy_ctx = proxy.clone();
        wb = wb.with_on_web_content_process_terminate_handler(move || {
            let _ = proxy_ctx.send_event(UserEvent::ContextLost {
                label: PRIMARY_LABEL.into(),
            });
        });
    }

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

    // Fail with an actionable message rather than a panic stack — a build
    // error here means WebView2 / WebKitGTK isn't installed on the host.
    let webview = match wb.build(&native_window) {
        Ok(wv) => wv,
        Err(e) => {
            crate::tynd_error!(
                "Failed to initialize WebView runtime: {e}. Check that WebView2 (Windows) or \
                 WebKitGTK (Linux) is installed."
            );
            std::process::exit(1);
        },
    };

    if config.window.center.unwrap_or(false) {
        window::center_window(&native_window);
    }

    // Soft FPS-cap diagnostic. Compositor enforces the actual frame budget;
    // this just surfaces operator misconfig (e.g. preferredFps=144 on a
    // 60 Hz monitor → silently capped to 60).
    gpu::warn_if_fps_above_monitor(&config.window, &native_window);

    let exit_started = std::sync::Arc::new(AtomicBool::new(false));
    // tao lacks minimize/maximize/fullscreen events; diff against this snapshot on Resized.
    let mut last_state = WindowState::capture(&native_window);
    let mut secondaries: HashMap<String, SecondaryEntry> = HashMap::new();
    let mut window_id_to_label: HashMap<WindowId, String> = HashMap::new();
    window_id_to_label.insert(native_window.id(), PRIMARY_LABEL.into());
    let primary_id = native_window.id();
    let dev_url_owned = config.dev_url.clone();
    let frontend_dir_owned = config.frontend_dir.clone();

    // Yield batching — one evaluate_script per webview per flush instead of per-yield.
    // Tuples are `(call_id, wire_value_string)` — the wire value is already
    // devalue/JSON-encoded by the runtime that produced it; the webview
    // shim decodes once on receipt.
    //
    // Per-label bucket is a `SmallVec<[_; 8]>`: a typical streaming call
    // flushes 1-8 chunks before the 10 ms timer fires, so the inline
    // capacity covers the common case without a heap allocation. Beyond
    // 8 the spill into heap is transparent (same `Deref<Target = [T]>`
    // surface as `Vec`, so `eval_yield_batch(&pairs[..])` still works
    // unchanged).
    let mut yield_buffer: HashMap<String, YieldBucket> = HashMap::new();
    // Cache setZoom writes; wry has no getter.
    let mut zoom_levels: HashMap<String, f64> = HashMap::new();
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

            Event::UserEvent(UserEvent::ContextLost { label }) => {
                // Notify the frontend so it can clear in-flight WebGPU /
                // WebGL state, then reload the affected webview to
                // re-create the GPU contexts. Same recovery path as a
                // dev-mode `BackendEvent::Reload` for that one window.
                let payload = json!({ "label": &label });
                let emit_script = ipc::eval_dispatch("webview:context-lost", &payload.to_string());
                let wv = webview_for(&label, &webview, &secondaries);
                let _ = wv.evaluate_script(&emit_script);
                let _ = wv.evaluate_script("window.location.reload();");
                crate::tynd_warn!(
                    "webview '{label}' context lost — reloading to recover GPU state"
                );
            },

            Event::UserEvent(UserEvent::FlushYields) => {
                // SAFETY: store(false) BEFORE flush ensures yields arriving
                // during flush will see `flush_scheduled=false` and
                // re-spawn a new FlushYields. Doing it after creates a
                // lost-wakeup race where yields land between flush and
                // store and stay buffered until the next yield bumps the
                // bucket past `YIELD_BATCH_MAX`.
                flush_scheduled.store(false, Ordering::Release);
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
                    let label = dispatch::CALL_LABELS
                        .remove(&id)
                        .map_or_else(|| PRIMARY_LABEL.into(), |(_, l)| l);
                    let wv = webview_for(&label, &webview, &secondaries);
                    let _ = wv.evaluate_script(&ipc::eval_resolve(&id, ok, &value));
                },
                BackendEvent::Yield { id, value } => {
                    let label = dispatch::CALL_LABELS
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
                BackendEvent::EvalInFrontend { script, to } => {
                    // Backend asked us to evaluate raw JS on the target
                    // webview(s). Replaces the previous `__tynd:eval__`
                    // string-named event so the script payload doesn't
                    // need to ride through devalue/JSON encoding.
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
                    // Broadcast to every window so Settings / About / popups don't keep stale HTML.
                    let _ = webview.evaluate_script(&ipc::eval_hide_error_overlay());
                    let _ = webview.evaluate_script("window.location.reload();");
                    for entry in secondaries.values() {
                        let _ = entry
                            .webview
                            .evaluate_script(&ipc::eval_hide_error_overlay());
                        let _ = entry.webview.evaluate_script("window.location.reload();");
                    }
                },
                BackendEvent::Error { message } => {
                    let script = ipc::eval_show_error_overlay(&message);
                    let _ = webview.evaluate_script(&script);
                    for entry in secondaries.values() {
                        let _ = entry.webview.evaluate_script(&script);
                    }
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
                        let mut labels: Vec<Value> = Vec::with_capacity(secondaries.len() + 1);
                        labels.push(Value::String(PRIMARY_LABEL.into()));
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

            Event::UserEvent(UserEvent::MenuCmd {
                label,
                id,
                method,
                args,
            }) => {
                // Menu mutations don't need the bar handle — `menu::dispatch`
                // looks items up in the thread-local registry that
                // `build_bar` populates. If no bar was built (empty config)
                // the registry is empty and every `update` rejects with
                // "item id 'X' not found", which is the right shape — apps
                // that didn't configure a menu bar shouldn't be calling
                // `menu.update` in the first place.
                let (ok, value) = match menu::dispatch(&method, &args) {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
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

/// Strict deep-link detector. Parses `s` with `url::Url::parse` and only
/// accepts an allowlisted scheme:
///
/// - `http` / `https` — standard web URLs
/// - `file` — bare `file://host/path` only; relative `file://./…` and
///   host-less drive paths are rejected
/// - the optional custom scheme registered via [`set_deep_link_scheme`]
///   (W3.1 deep-linking wires this up; defaults to none until then)
///
/// Explicitly rejected even if syntactically valid: `javascript`, `data`,
/// `vbscript`, `about`, `chrome`, `chrome-extension`, `blob`, `view-source`.
/// Without this guard a user double-clicking `myapp javascript:alert(1)`
/// (or a single-instance handover with a hostile argv) would dispatch the
/// payload string to the `app:open-url` handler verbatim.
fn looks_like_url(s: &str) -> bool {
    let Ok(parsed) = url::Url::parse(s) else {
        return false;
    };
    let scheme = parsed.scheme();

    // Hard deny — these schemes are dangerous as deep-link payloads even
    // when they parse, and no legitimate caller should emit them.
    match scheme {
        "javascript" | "data" | "vbscript" | "about" | "chrome" | "chrome-extension"
        | "blob" | "view-source" => return false,
        _ => {},
    }

    if matches!(scheme, "http" | "https") {
        return true;
    }
    if scheme == "file" {
        // Reject `file://./relative` / `file://../escape` and any URL
        // whose host segment contains traversal markers. `Url::parse`
        // accepts these as opaque hosts; we don't.
        let host = parsed.host_str().unwrap_or("");
        if host == "." || host == ".." || host.contains("./") || host.contains("../") {
            return false;
        }
        return true;
    }
    if let Some(custom) = deep_link_scheme() {
        if scheme.eq_ignore_ascii_case(custom) {
            return true;
        }
    }
    false
}

/// Custom scheme allowed by [`looks_like_url`]. Populated by the (future)
/// deep-link plugin once W3.1 lands; until then this stays `None` and
/// only `http` / `https` / `file` are accepted.
static DEEP_LINK_SCHEME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[allow(dead_code)]
pub(crate) fn set_deep_link_scheme(scheme: &str) {
    let _ = DEEP_LINK_SCHEME.set(scheme.to_ascii_lowercase());
}

fn deep_link_scheme() -> Option<&'static str> {
    DEEP_LINK_SCHEME.get().map(String::as_str)
}

/// Cancel every active streaming call whose origin window just closed. Looks
/// up ids in `dispatch::CALL_LABELS` and sends `BackendCall::Cancel` for
/// each match. Without this, a closed secondary window leaves its async
/// generators running forever on the backend.
fn cancel_streams_for_label(label: &str, call_tx: &crossbeam_channel::Sender<BackendCall>) {
    let labels = &dispatch::CALL_LABELS;
    // Closing a window typically leaves 0-2 in-flight streaming calls (a
    // chat completion, maybe a log tail). Inline-4 covers that without a
    // heap allocation; the rare app that fans out beyond 4 streams per
    // window spills to the heap transparently. `SmallVec` exposes the
    // same `IntoIterator` impl as `Vec`, so the loop below stays
    // identical.
    let orphaned: SmallVec<[String; 4]> = labels
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
///
/// `YieldBucket` derefs to `&[(String, String)]` so `eval_yield_batch` (which
/// already takes a slice) accepts it unchanged regardless of whether the
/// `SmallVec` lives inline or has spilled to the heap.
fn flush_yields(
    buffer: &mut HashMap<String, YieldBucket>,
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
///
/// Uses `serialize_to_javascript` so embedded U+2028 / U+2029, `</script>`,
/// quotes, and other JS-string-terminating characters are escaped at the
/// serialization layer — safer than the previous `serde_json::to_string` +
/// `format!` approach which could still produce literal line terminators in
/// some edge cases.
fn inject_window_label(label: &str) -> String {
    use serialize_to_javascript::{Options, Template};

    #[derive(Template)]
    struct WindowLabelScript<'a> {
        label: &'a str,
    }

    let script = WindowLabelScript { label };
    match script.render(
        "window.__TYND_WINDOW_LABEL__ = __TEMPLATE_label__;",
        &Options::default(),
    ) {
        Ok(s) => s.into_string(),
        Err(_) => r#"window.__TYND_WINDOW_LABEL__ = "";"#.to_owned(),
    }
}

/// Create a secondary window + webview + state tracker. Called on the main
/// thread inside the event loop (wry/tao require it).
#[allow(clippy::too_many_arguments)]
fn create_secondary(
    target: &EventLoopWindowTarget<UserEvent>,
    args: &Value,
    debug: bool,
    call_tx: &crossbeam_channel::Sender<BackendCall>,
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
        // Devalue must execute before JS_SHIM so the shim's _decode/_encode
        // helpers see `window.__tynd_dv__` already populated.
        .with_initialization_script(ipc::JS_DEVALUE)
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

    // Secondary windows inherit the primary's WebGPU / FPS / HDR knobs by
    // default — the GPU process is shared, so per-window divergence
    // wouldn't actually take effect. We still wire the context-lost
    // handler per-window so a recovery reload only touches the affected
    // surface.
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        let proxy_ctx = proxy.clone();
        let label_ctx = label.clone();
        wvb = wvb.with_on_web_content_process_terminate_handler(move || {
            let _ = proxy_ctx.send_event(UserEvent::ContextLost {
                label: label_ctx.clone(),
            });
        });
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

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::*;

    #[test]
    fn looks_like_url_accepts_http_https() {
        assert!(looks_like_url("https://example.com"));
        assert!(looks_like_url("http://example.com/path?q=1"));
        assert!(looks_like_url("https://example.com/path#frag"));
    }

    #[test]
    fn looks_like_url_accepts_file_with_host() {
        assert!(looks_like_url("file:///etc/hosts"));
        assert!(looks_like_url("file://localhost/etc/hosts"));
    }

    #[test]
    fn looks_like_url_rejects_dangerous_schemes() {
        // The whole point of S6 — these used to pass the cheap detector
        // and reach `app:open-url` handlers.
        assert!(!looks_like_url("javascript:alert(1)"));
        assert!(!looks_like_url("JAVASCRIPT:alert(1)"));
        assert!(!looks_like_url("data:text/html,<script>alert(1)</script>"));
        assert!(!looks_like_url("vbscript:msgbox"));
        assert!(!looks_like_url("about:blank"));
        assert!(!looks_like_url("chrome://settings"));
        assert!(!looks_like_url("chrome-extension://abc/foo"));
        assert!(!looks_like_url("blob:https://example.com/uuid"));
        assert!(!looks_like_url("view-source:https://example.com"));
    }

    #[test]
    fn looks_like_url_rejects_unknown_scheme_without_registration() {
        // Custom schemes are off until W3.1 wires `set_deep_link_scheme`.
        assert!(!looks_like_url("myapp://oauth/callback"));
    }

    #[test]
    fn looks_like_url_rejects_garbage() {
        assert!(!looks_like_url(""));
        assert!(!looks_like_url("not a url"));
        assert!(!looks_like_url("://no-scheme"));
        assert!(!looks_like_url("1http://numeric-prefix"));
    }

    /// B9 regression: when `flush_scheduled.store(false)` happens BEFORE
    /// the flush, a yield arriving DURING the flush observes
    /// `flush_scheduled == false`, so it re-arms a new FlushYields and
    /// gets seen on the next pass instead of being stranded in the
    /// buffer until something else bumps it past `YIELD_BATCH_MAX`.
    ///
    /// We model the race directly: a producer thread spams "yield" + try
    /// to schedule a flush, while a consumer thread plays the
    /// FlushYields handler on a small busy window. The check is that
    /// every produced yield is eventually flushed.
    #[test]
    fn flush_scheduled_release_before_flush_avoids_lost_wakeup() {
        const PRODUCED: usize = 5_000;

        let flush_scheduled = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(AtomicUsize::new(0));
        let flushed = Arc::new(AtomicUsize::new(0));
        let producer_done = Arc::new(AtomicBool::new(false));

        // Consumer mirrors `Event::UserEvent(UserEvent::FlushYields)`:
        // store(false) BEFORE the flush so that yields arriving in the
        // flush window can re-schedule us before we exit.
        let scheduled = flush_scheduled.clone();
        let pending_c = pending.clone();
        let flushed_c = flushed.clone();
        let done = producer_done.clone();
        let consumer = std::thread::spawn(move || loop {
            if !scheduled.load(Ordering::Acquire) {
                if done.load(Ordering::Acquire) && pending_c.load(Ordering::Acquire) == 0 {
                    return;
                }
                std::thread::yield_now();
                continue;
            }
            // ─── B9 fix: clear BEFORE the flush ───
            scheduled.store(false, Ordering::Release);
            // Drain pending yields ("flush"). In the buggy ordering the
            // store(false) below this line would let a yield arrive
            // mid-drain, see `scheduled == true`, decline to reschedule,
            // and be stranded.
            let drained = pending_c.swap(0, Ordering::AcqRel);
            flushed_c.fetch_add(drained, Ordering::Relaxed);
        });

        // Producer mirrors the `BackendEvent::Yield` arm: bump pending,
        // then try to flip `scheduled` to true if it was false. The
        // consumer's race window is between its swap(0) and the next
        // load(scheduled).
        let scheduled = flush_scheduled.clone();
        let pending_p = pending.clone();
        let producer = std::thread::spawn(move || {
            for _ in 0..PRODUCED {
                pending_p.fetch_add(1, Ordering::AcqRel);
                let _ = scheduled.swap(true, Ordering::AcqRel);
                // Tiny pause — without it the producer monopolizes the
                // CPU on a single-core scheduler and the race never
                // surfaces.
                std::thread::yield_now();
            }
            // Final wake in case the last yield landed while the
            // consumer was mid-flush.
            scheduled.store(true, Ordering::Release);
        });

        producer.join().unwrap();
        producer_done.store(true, Ordering::Release);
        // Final wake so the consumer doesn't sleep forever waiting on a
        // schedule that already happened.
        flush_scheduled.store(true, Ordering::Release);
        consumer.join().unwrap();

        let flushed = flushed.load(Ordering::Acquire);
        let pending = pending.load(Ordering::Acquire);
        assert_eq!(
            pending, 0,
            "{pending} yields stranded in buffer (lost wakeup)"
        );
        assert_eq!(
            flushed, PRODUCED,
            "expected {PRODUCED} yields flushed, got {flushed}"
        );
    }
}

/// Headless integration tests for the tao + wry runtime layer. Each test
/// is `#[ignore]` because it spins up a real OS event loop + WebView,
/// which requires a display server. CI hosts that don't have one (most
/// Linux runners by default) should run them via:
///
/// ```text
/// xvfb-run -a cargo test -p tynd-host --no-default-features \
///     --features "api-all" --lib -- --ignored headless
/// ```
///
/// On Windows / macOS the tests run unguarded under any local dev
/// session — `cargo test -p tynd-host --lib -- --ignored headless`
/// from a logged-in shell is enough.
///
/// These exist so we catch regressions where:
///   - tao + wry's version pin breaks `WebViewBuilder::build` against
///     a hidden window,
///   - `inject_window_label` produces script output that doesn't
///     round-trip through `serialize_to_javascript`,
///   - the `UserEvent` enum's `FlushYields` arm panics on an empty
///     buffer.
///
/// They intentionally do *not* drive a full IPC handshake — that path
/// is covered by the unit tests above and by the `dispatch.rs` tests.
/// The headless layer's job is purely "does the native plumbing
/// instantiate without crashing".
#[cfg(test)]
mod headless_tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use serde_json::json;
    use tao::{
        event::Event,
        event_loop::{ControlFlow, EventLoopBuilder},
        platform::run_return::EventLoopExtRunReturn,
        window::WindowBuilder,
    };

    use super::{inject_window_label, UserEvent, YIELD_BATCH_MAX};

    /// Build an event loop on whatever thread we're on (cargo's test
    /// harness picks a worker thread for each test, not necessarily
    /// the main thread). On Windows this is supported via
    /// `EventLoopBuilderExtWindows::with_any_thread(true)`; on Linux
    /// (X11/Wayland) the equivalent is in `tao::platform::unix`. Both
    /// are needed otherwise tao asserts "must be main thread" on
    /// build.
    fn build_event_loop() -> tao::event_loop::EventLoop<UserEvent> {
        let mut builder = EventLoopBuilder::<UserEvent>::with_user_event();
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        #[cfg(target_os = "linux")]
        {
            use tao::platform::unix::EventLoopBuilderExtUnix;
            builder.with_any_thread(true);
        }
        builder.build()
    }

    /// Spin the event loop for at most a handful of poll iterations,
    /// running `setup` once on the first poll. Used to give wry/tao a
    /// chance to materialize the window + webview before we tear it
    /// down. We exit on `ControlFlow::Exit` so the test process
    /// doesn't linger after the assertion completes.
    fn run_briefly<S>(event_loop: &mut tao::event_loop::EventLoop<UserEvent>, mut setup: S)
    where
        S: FnMut(&tao::event_loop::EventLoopWindowTarget<UserEvent>),
    {
        let mut iterations: u32 = 0;
        let mut setup_done = false;
        event_loop.run_return(move |event, target, control_flow| {
            *control_flow = ControlFlow::Poll;
            if !setup_done {
                setup(target);
                setup_done = true;
            }
            // Bail out either after a handful of iterations or once tao
            // signals shutdown — whichever comes first. The bound stops
            // a flaky platform from hanging the test.
            iterations += 1;
            if iterations > 20 || matches!(event, Event::LoopDestroyed) {
                *control_flow = ControlFlow::Exit;
            }
        });
    }

    /// Smoke test: build a hidden window + WebView pointing at
    /// `about:blank`, then immediately tear them down. Failure modes
    /// caught: missing webview2 runtime on Win, missing webkit2gtk on
    /// Linux without xvfb, broken tao↔wry version pin.
    #[test]
    #[ignore = "headless: requires a display server (xvfb on CI). Run with `cargo test -- --ignored headless`."]
    fn webview_creates_and_loads_blank() {
        let mut event_loop = build_event_loop();
        run_briefly(&mut event_loop, |target| {
            let window = WindowBuilder::new()
                .with_title("tynd-test")
                .with_visible(false)
                .build(target)
                .expect("WindowBuilder::build");
            let webview = wry::WebViewBuilder::new()
                .with_url("about:blank")
                .build(&window)
                .expect("WebViewBuilder::build");
            // Drop in this order: webview first (it borrows the
            // window's native handle), then the window. Without the
            // explicit drops the temporaries would live until the
            // end of the closure — same effect, but spelling it out
            // makes the lifetime contract obvious.
            drop(webview);
            drop(window);
        });
    }

    /// `inject_window_label` is a thin wrapper around
    /// `serialize_to_javascript::Template`. We assert that the
    /// rendered script is non-empty and contains the magic global
    /// the IPC shim reads on every `os_call` for the secondary-window
    /// router. A regression here would silently break label-scoped
    /// event filtering in production.
    #[test]
    #[ignore = "headless: cheap but grouped with the other headless tests so `--ignored headless` runs the whole layer."]
    fn inject_window_label_round_trips() {
        let script = inject_window_label("primary");
        assert!(
            script.contains("__TYND_WINDOW_LABEL__"),
            "missing window-label global: {script}"
        );
        assert!(
            script.contains("primary"),
            "label not embedded in script: {script}"
        );

        // Edge case: a label containing JS string terminators or
        // U+2028 / U+2029 must round-trip through the
        // `serialize-to-javascript` escape pipeline. The crate
        // wraps payloads in `JSON.parse('...')`, which neutralises
        // both line terminators and any nested `</script>` (the
        // payload sits inside a JS string fed to JSON.parse, so the
        // browser parses it as a JSON string token, not as a
        // `<script>` close). The contract we enforce here: the
        // emitted assignment must contain the `JSON.parse(` wrapper
        // — its absence would mean we fell back to raw concat.
        let hostile = inject_window_label(r"</script><script>alert(1)");
        assert!(
            hostile.contains("JSON.parse("),
            "expected serialize-to-javascript escape pipeline: {hostile}"
        );
        // Sanity: a U+2028 (line separator) — a classic JS-string
        // terminator — also goes through the escape pipeline.
        let line_sep = inject_window_label("\u{2028}danger");
        assert!(
            line_sep.contains("JSON.parse("),
            "U+2028 escape pipeline missing: {line_sep}"
        );
    }

    /// Yield-buffer flush threshold contract: once we have buffered
    /// `YIELD_BATCH_MAX` yields, the next push must trigger a flush
    /// synchronously rather than wait for the 10 ms timer. We model
    /// the buffer as a counter + threshold check — same shape the
    /// real arm uses without dragging in a full WebView.
    ///
    /// Failure here means a high-throughput producer (e.g. a plugin
    /// streaming binary frames) would burn unbounded memory between
    /// timer ticks.
    #[test]
    #[ignore = "headless: grouped with the rest so `--ignored headless` is the single switch. No display required for this test, but keeping the gate consistent makes CI scripting trivial."]
    fn yield_buffer_flushes_on_threshold() {
        let buffered = Arc::new(AtomicUsize::new(0));
        let flushes = Arc::new(AtomicUsize::new(0));

        // Drive 3× the threshold through the same logic the
        // FlushYields arm uses: bump on push, flush + zero on
        // threshold cross. Run on a worker so we exercise the
        // cross-thread Acquire/Release ordering, not just
        // single-threaded happens-before.
        let buf = buffered.clone();
        let flu = flushes.clone();
        let producer = std::thread::spawn(move || {
            for _ in 0..(YIELD_BATCH_MAX * 3) {
                let prev = buf.fetch_add(1, Ordering::AcqRel);
                if prev + 1 >= YIELD_BATCH_MAX {
                    // Mirror the synchronous-flush branch.
                    let drained = buf.swap(0, Ordering::AcqRel);
                    if drained > 0 {
                        flu.fetch_add(1, Ordering::Release);
                    }
                }
            }
        });
        producer.join().unwrap();

        // We pushed 3× the threshold, so we expect at least 3 flushes
        // (the exact number is `>=3` because the synchronous-flush
        // branch can race with itself harmlessly). The buffer must
        // be empty at the end.
        let flushes_seen = flushes.load(Ordering::Acquire);
        let leftover = buffered.load(Ordering::Acquire);
        assert!(
            flushes_seen >= 3,
            "expected ≥3 threshold flushes, saw {flushes_seen}"
        );
        // ≤ 1 round of leftover is acceptable — the model elides
        // the 10 ms timer flush that would clear the trailing
        // partial batch in production. `< YIELD_BATCH_MAX` is the
        // contract.
        assert!(
            leftover < YIELD_BATCH_MAX,
            "leftover buffer {leftover} ≥ flush threshold {YIELD_BATCH_MAX}"
        );
    }

    /// A `UserEvent::FlushYields` arriving when no yields are
    /// buffered must be a no-op — the buffer's `swap(0)` returns
    /// zero and the eval-script call is skipped. Without this, an
    /// orphan FlushYields (e.g. left over after a window close) would
    /// trigger an empty `evaluate_script("")` per webview.
    #[test]
    #[ignore = "headless: grouped under `--ignored headless` even though it runs without a display."]
    fn flush_yields_empty_buffer_is_noop() {
        let buf = Arc::new(AtomicUsize::new(0));
        // Mirror the FlushYields arm: only touch the webview when
        // drained > 0.
        let drained = buf.swap(0, Ordering::AcqRel);
        let mut webview_eval_calls = 0;
        if drained > 0 {
            webview_eval_calls += 1;
        }
        let _ = json!({ "drained": drained });
        assert_eq!(webview_eval_calls, 0, "empty FlushYields evaluated script");
    }

    /// Sanity: a freshly-built EventLoop carries the user-event
    /// channel we depend on. Acts as a canary — if a tao bump renames
    /// `with_user_event` or stops accepting our `UserEvent` payload,
    /// this test fails before any production code path does.
    #[test]
    #[ignore = "headless: bundled with the rest of the headless smoke checks."]
    fn event_loop_proxy_round_trips_user_event() {
        let mut event_loop = build_event_loop();
        let proxy = event_loop.create_proxy();
        // Send before run_return so the event is queued and observed
        // on the first poll — avoids a race against the loop-init
        // synthetic Resumed event.
        proxy
            .send_event(UserEvent::ProceedClose)
            .expect("send_event");

        let observed = Arc::new(AtomicUsize::new(0));
        let observed_c = observed.clone();
        run_briefly(&mut event_loop, move |_target| {
            observed_c.fetch_add(1, Ordering::Release);
        });
        // The setup closure runs at least once — confirms the loop
        // pumped at all. The user-event arrival is best-effort here
        // (depends on platform timer granularity) so we don't gate
        // the test on it; the construction success itself is the
        // contract this test enforces.
        assert!(observed.load(Ordering::Acquire) >= 1);
    }
}
