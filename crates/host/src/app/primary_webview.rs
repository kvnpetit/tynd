use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

#[cfg(target_os = "linux")]
use super::LinuxWebviewContainer;
use super::{gpu, util::webview_data_dir, UserEvent, PRIMARY_LABEL};
use crate::webview::{
    builder::{with_debug_tools, with_tynd_runtime},
    handlers::{apply_webview_policy, WebviewPolicy},
};
use tynd_protocol::runtime::{BackendCall, BackendConfig};

use crate::{
    webview::{scheme, scheme_bin},
    window,
};

pub(super) struct PrimaryWebview {
    pub(super) app_name: String,
    pub(super) webview: WebView,
    // Drop the WebView before its context and Linux composition container.
    pub(super) _web_context: WebContext,
    #[cfg(target_os = "linux")]
    pub(super) container: LinuxWebviewContainer,
}

fn configure_primary_content<'a>(
    mut builder: WebViewBuilder<'a>,
    config: &BackendConfig,
    app_name: &str,
) -> WebViewBuilder<'a> {
    if let Some(ref url) = config.dev_url {
        builder = builder.with_url(url);
    } else if let Some(ref dir) = config.frontend_dir {
        let dir = dir.clone();
        builder = builder
            .with_custom_protocol("tynd".into(), move |_id, request: Request<Vec<u8>>| {
                scheme::handle(&dir, &request)
            })
            .with_url("tynd://localhost/");
    } else {
        builder = builder.with_html(window::placeholder_html(app_name));
    }
    builder
}
pub(super) fn build_primary_webview(
    config: &BackendConfig,
    native_window: &Window,
    call_tx: &crossbeam_channel::Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
    debug: bool,
    debug_protocol_enabled: bool,
) -> Result<PrimaryWebview, String> {
    #[cfg(not(feature = "debug-protocol"))]
    let _ = debug_protocol_enabled;

    // Store WebView2 / WKWebView data in the OS app-data dir, not next to the exe.
    let default_app_name: String = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "tynd".to_string());
    let app_name = config.window.title.as_deref().unwrap_or(&default_app_name);
    let webview_app_name = app_name.to_string();
    let mut web_context = WebContext::new(webview_data_dir(app_name));
    #[cfg(target_os = "linux")]
    let primary_webview_container =
        LinuxWebviewContainer::new(&native_window).unwrap_or_else(|e| {
            crate::app::exit_startup_failure(&format!(
                "Failed to initialize Linux GTK WebView container: {e}"
            ))
        });

    let policy = WebviewPolicy::for_primary(&config.window)?;
    let protocol_id = scheme_bin::renderer_id(PRIMARY_LABEL, PRIMARY_LABEL);
    let wb = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_id(&protocol_id)
        .with_incognito(config.window.incognito.unwrap_or(false));
    let mut wb = with_tynd_runtime(wb, PRIMARY_LABEL, PRIMARY_LABEL, call_tx, proxy);

    // Debug helper — only injected when the debug protocol is on, so
    // production webviews don't carry the dev-mode fetch/XHR proxies.
    #[cfg(feature = "debug-protocol")]
    if debug_protocol_enabled {
        wb = wb.with_initialization_script(crate::debug::DEBUG_HELPER_JS);
    }

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
        wb = crate::webview::builder::with_context_lost_handler(wb, proxy, PRIMARY_LABEL.into());
    }

    if let Some(ref ua) = config.window.user_agent {
        wb = wb.with_user_agent(ua);
    }

    // Bundled or dev-server content: app-local by definition.
    wb = apply_webview_policy(wb, PRIMARY_LABEL, &policy, None);
    wb = with_debug_tools(wb, debug);

    // Binary IPC is needed in both dev and prod so `fs.readBinary` etc.
    // keep working against the dev server. Async so disk IO and multi-MB
    // compression run on the call pool instead of the UI thread.
    wb = wb.with_asynchronous_custom_protocol(
        "tynd-bin".into(),
        |id, req: Request<Vec<u8>>, responder| scheme_bin::handle_async(id, req, responder),
    );

    wb = configure_primary_content(wb, config, app_name);

    // Fail with an actionable message rather than a panic stack — a build
    // error here means WebView2 / WebKitGTK isn't installed on the host.
    #[cfg(not(target_os = "linux"))]
    let webview_result = wb.build(&native_window);
    #[cfg(target_os = "linux")]
    let webview_result = primary_webview_container.build_root(wb);
    let webview = webview_result.map_err(|error| {
        format!(
            "Failed to initialize WebView runtime: {error}. Check that WebView2 (Windows) or \
             WebKitGTK (Linux) is installed."
        )
    })?;

    Ok(PrimaryWebview {
        app_name: webview_app_name,
        webview,
        _web_context: web_context,
        #[cfg(target_os = "linux")]
        container: primary_webview_container,
    })
}
