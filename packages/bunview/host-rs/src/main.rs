// Hide console window in Windows release builds
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// mimalloc — faster allocation than system malloc for JSON parsing, sysinfo, etc.
// 10-30% speedup on the monitor thread's hot loop.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod args;
mod cmds;
mod ipc;
mod proto;
mod scheme;
mod state;

use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    window::WindowBuilder,
};
use wry::{http::Request, WebViewBuilder};
#[cfg(target_os = "windows")]
use wry::WebViewBuilderExtWindows;

use args::Args;
use proto::InboundMsg;
use state::AppState;

fn main() {
    let args = Args::parse();
    let event_loop = EventLoopBuilder::<InboundMsg>::with_user_event().build();
    let proxy      = event_loop.create_proxy();
    let mut wb = WindowBuilder::new()
        .with_title(&args.title)
        .with_inner_size(LogicalSize::new(args.width, args.height))
        .with_resizable(args.resizable)
        .with_decorations(!args.frameless)
        .with_transparent(args.transparent)
        // Start hidden — shown by ShowWindow event once the real page is rendered.
        // This eliminates the white flash from the blank placeholder HTML.
        .with_visible(false);

    if let Some(ref icon_path) = args.icon {
        match load_window_icon(icon_path) {
            Ok(icon) => { wb = wb.with_window_icon(Some(icon)); }
            Err(e)   => eprintln!("[bunview] icon load failed: {e}"),
        }
    }

    let window = wb.build(&event_loop).expect("WindowBuilder::build");

    // Apply initial vibrancy if requested
    if let Some(ref v) = args.vibrancy {
        cmds::apply_vibrancy_initial(&window, v);
    }
    let static_dir = args.static_dir.clone();

    // wry 0.55: WebViewBuilder::new() then .build(&window)
    let proxy_ipc = proxy.clone();
    let mut builder = WebViewBuilder::new()
        .with_initialization_script(ipc::JS_PAGE_READY)
        .with_initialization_script(ipc::JS_SHIM)
        .with_initialization_script(ipc::CLIENT_SCRIPT)
        .with_ipc_handler(move |req: Request<String>| {
            if ipc::is_page_ready(req.body()) {
                // Wake up the event loop to show the window
                let _ = proxy_ipc.send_event(InboundMsg::ShowWindow);
            } else {
                ipc::handle_ipc(req.body());
            }
        });

    if args.debug {
        builder = builder.with_devtools(true);
    }

    // Hardware acceleration browser args — WebGPU + WebNN when opted-in.
    // WebNN routes to NPU via DirectML (Windows) or CoreML/ANE (macOS).
    #[cfg(target_os = "windows")]
    {
        let browser_args = if args.hardware_acceleration {
            // D3D12 + DirectML backend:
            //   WebGPU  → GPU compute / rendering
            //   WebNN   → Neural network API; uses DirectML backend on Windows,
            //             which automatically targets GPU, Intel NPU (AI Boost),
            //             AMD XDNA (Ryzen AI), or Qualcomm NPU via ONNX Runtime.
            "--use-angle=d3d12 \
             --enable-gpu-rasterization \
             --enable-zero-copy \
             --enable-features=WebGPU,WebMachineLearningNeuralNetwork"
        } else {
            "--disable-gpu \
             --disable-gpu-compositing \
             --disable-gpu-rasterization \
             --disable-features=WebGPU,WebMachineLearningNeuralNetwork"
        };
        builder = builder.with_additional_browser_args(browser_args);
    }

    if let Some(ref dir) = static_dir {
        let dir = dir.clone();
        // wry 0.55: custom protocol handler receives (webview_id, request)
        builder = builder.with_custom_protocol("bv".into(), move |_id, req: Request<Vec<u8>>| {
            scheme::handle(&dir, req)
        });
    }

    // Start with a blank page; Bun sends navigate right after ready
    builder = builder.with_html("<html><body style='margin:0'></body></html>");

    let webview = builder.build(&window).expect("WebViewBuilder::build");
    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                match line {
                    Ok(l) if !l.trim().is_empty() => {
                        match serde_json::from_str::<InboundMsg>(&l) {
                            Ok(msg) => { let _ = proxy.send_event(msg); }
                            Err(e)  => eprintln!("[bunview:host] Parse error ({e}): {l}"),
                        }
                    }
                    Err(_) => {
                        // EOF → parent Bun process exited
                        std::process::exit(0);
                    }
                    _ => {} // empty line
                }
            }
            std::process::exit(0);
        });
    }
    let mut state = AppState::new();

    // Signal Bun that we are ready
    ipc::emit_ready();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        // Poll cross-thread event receivers every iteration
        cmds::poll_events(&mut state);

        match event {
            Event::UserEvent(msg) => {
                cmds::dispatch(msg, &window, &webview, &mut state);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                window.set_visible(false);
                ipc::emit_close();
                // std::process::exit skips tao's window teardown sequence entirely.
                // ControlFlow::Exit would destroy the window through the message pump,
                // causing WebView2 to repaint white before the OS removes it.
                std::process::exit(0);
            }
            Event::WindowEvent { event: WindowEvent::Moved(pos), .. } => {
                ipc::emit(serde_json::json!({
                    "type": "windowMoved",
                    "x": pos.x,
                    "y": pos.y,
                }));
            }
            Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
                let logical: LogicalSize<f64> = size.to_logical(window.scale_factor());
                ipc::emit(serde_json::json!({
                    "type":   "windowResized",
                    "width":  logical.width  as u32,
                    "height": logical.height as u32,
                }));
            }
            Event::WindowEvent { event: WindowEvent::Focused(focused), .. } => {
                ipc::emit(serde_json::json!({
                    "type":    "windowFocusChanged",
                    "focused": focused,
                }));
            }
            Event::WindowEvent { event: WindowEvent::DroppedFile(path), .. } => {
                ipc::emit(serde_json::json!({
                    "type":  "fileDrop",
                    "paths": [path.to_string_lossy().to_string()],
                }));
            }
            Event::WindowEvent { event: WindowEvent::HoveredFile(path), .. } => {
                ipc::emit(serde_json::json!({
                    "type":  "fileDragEnter",
                    "paths": [path.to_string_lossy().to_string()],
                }));
            }
            Event::WindowEvent { event: WindowEvent::HoveredFileCancelled, .. } => {
                ipc::emit(serde_json::json!({ "type": "fileDragLeave" }));
            }
            Event::WindowEvent { event: WindowEvent::ThemeChanged(theme), .. } => {
                ipc::emit(serde_json::json!({
                    "type":  "themeChanged",
                    "theme": match theme {
                        tao::window::Theme::Dark => "dark",
                        _ => "light",
                    },
                }));
            }

            _ => {}
        }
    });
}

fn load_window_icon(path: &str) -> Result<tao::window::Icon, Box<dyn std::error::Error>> {
    let img    = image::open(path)?.into_rgba8();
    let (w, h) = img.dimensions();
    let rgba   = img.into_raw();
    Ok(tao::window::Icon::from_rgba(rgba, w, h)?)
}
