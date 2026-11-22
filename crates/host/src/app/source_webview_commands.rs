//! Commands targeting the webview that originated a window IPC request.

use serde_json::Value;
use wry::WebView;

use super::{dispatch_support::parse_zoom_level, runtime::WindowRegistry, PRIMARY_LABEL};
use crate::webview::navigation::resolve_navigation_url;

type CommandResult = Result<Value, String>;

pub(super) struct SourceWebviewCommandContext<'a> {
    pub(super) windows: &'a mut WindowRegistry,
}

pub(super) fn dispatch_source_webview_command(
    method: &str,
    label: &str,
    args: &Value,
    mut context: SourceWebviewCommandContext<'_>,
) -> Option<CommandResult> {
    Some(match method {
        "reload" => Ok(reload_source_webview(&context, label)),
        "setZoom" => set_source_webview_zoom(&mut context, label, args),
        "getZoom" => Ok(source_webview_zoom(&context, label)),
        "openDevTools" => open_source_webview_devtools(&context, label),
        "closeDevTools" => Ok(close_source_webview_devtools(&context, label)),
        "print" => print_source_webview(&context, label),
        "navigate" => navigate_source_webview(&context, label, args),
        "loadHtml" => load_source_webview_html(&context, label, args),
        "getUrl" => source_webview_url(&context, label),
        _ => return None,
    })
}
fn command_source_webview<'a>(label: &str, windows: &'a WindowRegistry) -> &'a WebView {
    windows
        .webview_for(label)
        .expect("window command source checked above")
}

fn reload_source_webview(context: &SourceWebviewCommandContext<'_>, label: &str) -> Value {
    let webview = command_source_webview(label, context.windows);
    let _ = webview.evaluate_script("window.location.reload();");
    Value::Null
}

fn set_source_webview_zoom(
    context: &mut SourceWebviewCommandContext<'_>,
    label: &str,
    args: &Value,
) -> CommandResult {
    let level = parse_zoom_level(args, "setZoom")?;
    let webview = command_source_webview(label, context.windows);
    webview
        .zoom(level)
        .map_err(|error| format!("setZoom: {error}"))?;
    context.windows.zoom_levels.insert(label.to_string(), level);
    Ok(Value::Null)
}

fn source_webview_zoom(context: &SourceWebviewCommandContext<'_>, label: &str) -> Value {
    Value::from(
        context
            .windows
            .zoom_levels
            .get(label)
            .copied()
            .unwrap_or(1.0),
    )
}

fn open_source_webview_devtools(
    context: &SourceWebviewCommandContext<'_>,
    label: &str,
) -> CommandResult {
    let webview = command_source_webview(label, context.windows);
    #[cfg(debug_assertions)]
    {
        webview.open_devtools();
        Ok(Value::Null)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = webview;
        Err("openDevTools: only available in debug builds".into())
    }
}

fn close_source_webview_devtools(context: &SourceWebviewCommandContext<'_>, label: &str) -> Value {
    let webview = command_source_webview(label, context.windows);
    #[cfg(debug_assertions)]
    webview.close_devtools();
    let _ = webview;
    Value::Null
}

fn print_source_webview(context: &SourceWebviewCommandContext<'_>, label: &str) -> CommandResult {
    let webview = command_source_webview(label, context.windows);
    webview.print().map_err(|error| format!("print: {error}"))?;
    Ok(Value::Null)
}

fn navigate_source_webview(
    context: &SourceWebviewCommandContext<'_>,
    label: &str,
    args: &Value,
) -> CommandResult {
    let url = args.get("url").and_then(Value::as_str).unwrap_or("");
    if url.is_empty() {
        return Err("navigate: missing 'url'".into());
    }
    let webview = command_source_webview(label, context.windows);
    let current = webview
        .url()
        .map_err(|error| format!("navigate: current URL: {error}"))?;
    let resolved =
        resolve_navigation_url(url, &current).map_err(|error| format!("navigate: {error}"))?;
    webview
        .load_url(&resolved)
        .map_err(|error| format!("navigate: {error}"))?;
    Ok(Value::Null)
}

fn load_source_webview_html(
    context: &SourceWebviewCommandContext<'_>,
    label: &str,
    args: &Value,
) -> CommandResult {
    // Arbitrary HTML inherits the webview's `__tynd__` shim, so it
    // must be opt-in per window via `allowLoadHtml: true` at create.
    // The primary window is never permitted — its origin is bundled.
    let allowed = label != PRIMARY_LABEL
        && context
            .windows
            .secondaries
            .get(label)
            .is_some_and(|entry| entry.allow_load_html);
    if !allowed {
        return Err("loadHtml: not permitted (set `allowLoadHtml: true` at window create)".into());
    }
    let html = args.get("html").and_then(Value::as_str).unwrap_or("");
    let webview = command_source_webview(label, context.windows);
    webview
        .load_html(html)
        .map_err(|error| format!("loadHtml: {error}"))?;
    Ok(Value::Null)
}

fn source_webview_url(context: &SourceWebviewCommandContext<'_>, label: &str) -> CommandResult {
    let webview = command_source_webview(label, context.windows);
    let url = webview.url().map_err(|error| format!("getUrl: {error}"))?;
    Ok(Value::String(url))
}
