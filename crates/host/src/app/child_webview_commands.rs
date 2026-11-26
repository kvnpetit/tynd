//! Child-webview command routing and state mutation with a narrow command context.

use crossbeam_channel::Sender;
use serde_json::{json, Value};
use tao::event_loop::EventLoopProxy;
use wry::WebView;

use super::{
    child_webview::{
        build_child_webview, parse_child_bounds, parse_child_webview_request, update_auto_resize,
        ChildWebviewParent,
    },
    dispatch_support::{
        child_webview_operation, parse_zoom_level, requested_label, window_for_label,
    },
    runtime::WindowRegistry,
    UserEvent, PRIMARY_LABEL,
};
use crate::webview::{
    handlers::{cookie_from_json, cookie_key_from_json, cookie_to_json, parse_http_url},
    navigation::resolve_navigation_url,
};
use tynd_protocol::runtime::BackendCall;

use crate::api;

type CommandResult = Result<Value, String>;

pub(super) struct ChildWebviewCommandContext<'a> {
    pub(super) debug: bool,
    /// Label of the webview that issued the command.
    pub(super) caller_webview: &'a str,
    /// Origin of the calling frame (`None` = app-local content). Authoritative
    /// for cookie scoping: it is the frame that issued the call, which can
    /// differ from the tracked main-frame origin (a remote iframe in local
    /// content).
    pub(super) caller_origin: Option<&'a str>,
    pub(super) call_tx: &'a Sender<BackendCall>,
    pub(super) proxy: &'a EventLoopProxy<UserEvent>,
    pub(super) dev_url: Option<&'a str>,
    pub(super) frontend_dir: Option<&'a str>,
    pub(super) windows: &'a mut WindowRegistry,
}

/// Every method this module routes. Cookie methods are included: they are
/// capability-checked once, centrally, in `dispatch::handle_os_call` with the
/// real window and webview labels (an explicit grant is always required).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildMethod {
    Create,
    Close,
    All,
    SetBounds,
    SetVisible,
    SetAutoResize,
    Focus,
    FocusParent,
    Reload,
    Navigate,
    GetUrl,
    SetZoom,
    OpenDevTools,
    CloseDevTools,
    Print,
    ClearData,
    GetCookies,
    SetCookie,
    DeleteCookie,
    ClearCookies,
}

const CHILD_METHODS: &[(&str, ChildMethod)] = &[
    ("createWebview", ChildMethod::Create),
    ("closeWebview", ChildMethod::Close),
    ("allWebviews", ChildMethod::All),
    ("setWebviewBounds", ChildMethod::SetBounds),
    ("setWebviewVisible", ChildMethod::SetVisible),
    ("setWebviewAutoResize", ChildMethod::SetAutoResize),
    ("focusWebview", ChildMethod::Focus),
    ("focusParentWebview", ChildMethod::FocusParent),
    ("reloadWebview", ChildMethod::Reload),
    ("navigateWebview", ChildMethod::Navigate),
    ("getWebviewUrl", ChildMethod::GetUrl),
    ("setWebviewZoom", ChildMethod::SetZoom),
    ("openWebviewDevTools", ChildMethod::OpenDevTools),
    ("closeWebviewDevTools", ChildMethod::CloseDevTools),
    ("printWebview", ChildMethod::Print),
    ("clearWebviewData", ChildMethod::ClearData),
    ("getWebviewCookies", ChildMethod::GetCookies),
    ("setWebviewCookie", ChildMethod::SetCookie),
    ("deleteWebviewCookie", ChildMethod::DeleteCookie),
    ("clearWebviewCookies", ChildMethod::ClearCookies),
];

fn parse_method(method: &str) -> Option<ChildMethod> {
    CHILD_METHODS
        .iter()
        .find(|(name, _)| *name == method)
        .map(|(_, parsed)| *parsed)
}

impl ChildMethod {
    fn is_cookie(self) -> bool {
        matches!(
            self,
            Self::GetCookies | Self::SetCookie | Self::DeleteCookie | Self::ClearCookies
        )
    }

    /// Child webviews belong to the `multiwindow` API surface. Cookie methods
    /// also target the primary and secondary webviews, so they work without it.
    fn requires_multiwindow(self) -> bool {
        !self.is_cookie()
    }
}

pub(super) fn dispatch_child_webview_command(
    method: &str,
    source_window_label: &str,
    args: &Value,
    mut context: ChildWebviewCommandContext<'_>,
) -> Option<CommandResult> {
    let parsed = parse_method(method)?;
    if parsed.requires_multiwindow() {
        if let Err(error) = api::require_multiwindow() {
            return Some(Err(error));
        }
    }
    Some(match parsed {
        ChildMethod::Create => create_child_webview(&mut context, source_window_label, args),
        ChildMethod::Close => close_child_webview(&context, args),
        ChildMethod::All => Ok(webview_descriptors(&context)),
        ChildMethod::SetBounds => set_child_webview_bounds(&mut context, args),
        ChildMethod::SetVisible => set_child_webview_visible(&mut context, args),
        ChildMethod::SetAutoResize => set_child_webview_auto_resize(&mut context, args),
        ChildMethod::Focus => child_webview_operation(
            args,
            "focusWebview",
            &context.windows.child_webviews,
            WebView::focus,
        ),
        ChildMethod::FocusParent => child_webview_operation(
            args,
            "focusParentWebview",
            &context.windows.child_webviews,
            WebView::focus_parent,
        ),
        ChildMethod::Reload => child_webview_operation(
            args,
            "reloadWebview",
            &context.windows.child_webviews,
            |webview| webview.evaluate_script("window.location.reload();"),
        ),
        ChildMethod::Navigate => navigate_child_webview(&context, args),
        ChildMethod::GetUrl => child_webview_url(&context, args),
        ChildMethod::SetZoom => set_child_webview_zoom(&mut context, args),
        ChildMethod::OpenDevTools => open_child_webview_devtools(&context, args),
        ChildMethod::CloseDevTools => close_child_webview_devtools(&context, args),
        ChildMethod::Print => child_webview_operation(
            args,
            "printWebview",
            &context.windows.child_webviews,
            WebView::print,
        ),
        ChildMethod::ClearData => child_webview_operation(
            args,
            "clearWebviewData",
            &context.windows.child_webviews,
            WebView::clear_all_browsing_data,
        ),
        ChildMethod::GetCookies
        | ChildMethod::SetCookie
        | ChildMethod::DeleteCookie
        | ChildMethod::ClearCookies => cookie_command(method, args, &context),
    })
}

/// A webview may touch its own cookies. Reaching another webview's cookies
/// (HttpOnly session tokens included) needs a caller that shows app-local
/// content: a remote page holding a cookie grant must not read the app's.
fn check_cookie_target(
    caller: &str,
    target: &str,
    operation: &str,
    is_local: impl Fn(&str) -> bool,
) -> Result<(), String> {
    if caller == target || is_local(caller) {
        Ok(())
    } else {
        Err(format!(
            "{operation}: webview '{caller}' shows remote content and may only access its own \
             cookies"
        ))
    }
}

/// Webviews without incognito / `dataPartition` share one cookie store with
/// the app's own pages, so "its own cookies" is not enough for a remote
/// caller: it may only touch cookies of its own origin. It must name a `url`
/// on that origin (no store-wide list/clear) and any cookie `domain` must be
/// that host or one of its subdomains.
fn check_remote_cookie_scope(
    caller_origin: &str,
    method: &str,
    base_url: Option<&url::Url>,
    cookie_domain: Option<&str>,
) -> Result<(), String> {
    let url = base_url.ok_or_else(|| {
        format!("{method}: a webview showing remote content must pass the 'url' of its own origin")
    })?;
    if url.origin().ascii_serialization() != caller_origin {
        return Err(format!(
            "{method}: url must be on the webview's own origin ({caller_origin})"
        ));
    }
    if let (Some(domain), Some(host)) = (cookie_domain, url.host_str()) {
        let domain = domain.trim_start_matches('.').to_ascii_lowercase();
        let host = host.to_ascii_lowercase();
        let inside = domain == host || domain.ends_with(&format!(".{host}"));
        if !inside {
            return Err(format!(
                "{method}: cookie domain '{domain}' is outside the webview's own host"
            ));
        }
    }
    Ok(())
}

fn cookie_command(
    method: &str,
    args: &Value,
    context: &ChildWebviewCommandContext<'_>,
) -> CommandResult {
    let target_label = requested_label(args, method)?;
    check_cookie_target(context.caller_webview, target_label, method, |_| {
        context.caller_origin.is_none()
    })?;
    let webview = context
        .windows
        .webview_for(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?;
    let base_url = args
        .get("url")
        .and_then(Value::as_str)
        .map(|raw| parse_http_url(raw, method))
        .transpose()?;
    if let Some(caller_origin) = context.caller_origin {
        let domain_holder = if method == "setWebviewCookie" {
            args.get("cookie")
        } else {
            Some(args)
        };
        let cookie_domain = domain_holder
            .and_then(|holder| holder.get("domain"))
            .and_then(Value::as_str);
        check_remote_cookie_scope(caller_origin, method, base_url.as_ref(), cookie_domain)?;
    }
    match method {
        "getWebviewCookies" => {
            let cookies = list_cookies(webview, base_url.as_ref(), method)?;
            Ok(Value::Array(cookies.iter().map(cookie_to_json).collect()))
        },
        "setWebviewCookie" => {
            let object = args
                .get("cookie")
                .filter(|value| value.is_object())
                .ok_or_else(|| format!("{method}: missing 'cookie' object"))?;
            let cookie = cookie_from_json(object, base_url.as_ref(), method)?;
            webview
                .set_cookie(&cookie)
                .map_err(|error| format!("{method}: {error}"))?;
            Ok(Value::Null)
        },
        "deleteWebviewCookie" => {
            let cookie = cookie_key_from_json(args, base_url.as_ref(), method)?;
            webview
                .delete_cookie(&cookie)
                .map_err(|error| format!("{method}: {error}"))?;
            Ok(Value::Null)
        },
        _ => {
            let cookies = list_cookies(webview, base_url.as_ref(), method)?;
            let mut removed = 0_u64;
            for cookie in &cookies {
                webview
                    .delete_cookie(cookie)
                    .map_err(|error| format!("{method}: {error}"))?;
                removed += 1;
            }
            Ok(json!(removed))
        },
    }
}

fn list_cookies(
    webview: &WebView,
    url: Option<&url::Url>,
    operation: &str,
) -> Result<Vec<wry::cookie::Cookie<'static>>, String> {
    match url {
        Some(url) => webview.cookies_for_url(url.as_str()),
        None => webview.cookies(),
    }
    .map_err(|error| format!("{operation}: {error}"))
}

fn create_child_webview(
    context: &mut ChildWebviewCommandContext<'_>,
    source_window_label: &str,
    args: &Value,
) -> CommandResult {
    let request = parse_child_webview_request(args, source_window_label)?;
    if context.windows.secondaries.contains_key(&request.label)
        || context.windows.child_webviews.contains_key(&request.label)
    {
        return Err(format!(
            "createWebview: label '{}' already exists",
            request.label
        ));
    }
    let parent = window_for_label(
        &request.parent_label,
        &context.windows.native_window,
        &context.windows.secondaries,
    )
    .ok_or_else(|| {
        format!(
            "createWebview: parent window '{}' not found",
            request.parent_label
        )
    })?;
    #[cfg(target_os = "linux")]
    let parent_container = if request.parent_label == PRIMARY_LABEL {
        &context.windows.primary_webview_container
    } else {
        &context
            .windows
            .secondaries
            .get(&request.parent_label)
            .ok_or_else(|| {
                format!(
                    "createWebview: parent window '{}' not found",
                    request.parent_label
                )
            })?
            .webview_container
    };
    let parent = ChildWebviewParent {
        window: parent,
        #[cfg(target_os = "linux")]
        container: parent_container,
    };
    ensure_partition_available(context, request.data_partition.as_deref())?;

    let (new_label, entry) = build_child_webview(
        request,
        parent,
        context.debug,
        context.call_tx,
        context.proxy,
        context.dev_url,
        context.frontend_dir,
        &context.windows.webview_app_name,
    )?;
    let parent_label = entry.parent_label.clone();
    context
        .windows
        .child_webviews
        .insert(new_label.clone(), entry);
    crate::ipc::events::emit(
        "webview:created",
        &json!({ "label": &new_label, "parent": parent_label }),
    );
    Ok(Value::Null)
}

fn ensure_partition_available(
    context: &ChildWebviewCommandContext<'_>,
    partition: Option<&str>,
) -> Result<(), String> {
    let Some(partition) = partition else {
        return Ok(());
    };
    let already_open = context
        .windows
        .secondaries
        .values()
        .any(|entry| entry.data_partition.as_deref() == Some(partition))
        || context
            .windows
            .child_webviews
            .values()
            .any(|entry| entry.data_partition.as_deref() == Some(partition));
    if already_open {
        return Err(format!(
            "createWebview: dataPartition '{partition}' is already used by an open webview"
        ));
    }
    Ok(())
}

fn close_child_webview(context: &ChildWebviewCommandContext<'_>, args: &Value) -> CommandResult {
    let target_label = requested_label(args, "closeWebview")?;
    if !context.windows.child_webviews.contains_key(target_label) {
        return Err(format!("webview '{target_label}' not found"));
    }
    context
        .proxy
        .send_event(UserEvent::CloseChildWebview {
            label: target_label.to_string(),
        })
        .map_err(|error| format!("closeWebview: queue close: {error}"))?;
    Ok(Value::Null)
}

fn webview_descriptors(context: &ChildWebviewCommandContext<'_>) -> Value {
    let mut items = Vec::with_capacity(
        context.windows.secondaries.len() + context.windows.child_webviews.len() + 1,
    );
    items.push(json!({
        "label": PRIMARY_LABEL,
        "parent": Value::Null,
        "kind": "window"
    }));
    items.extend(context.windows.secondaries.keys().map(|label| {
        json!({
            "label": label,
            "parent": Value::Null,
            "kind": "window"
        })
    }));
    items.extend(context.windows.child_webviews.iter().map(|(label, entry)| {
        json!({
            "label": label,
            "parent": &entry.parent_label,
            "kind": "child",
            "bounds": {
                "x": entry.bounds.x,
                "y": entry.bounds.y,
                "width": entry.bounds.width,
                "height": entry.bounds.height,
            },
            "visible": entry.visible,
            "autoResize": entry.auto_resize,
            "dataPartition": &entry.data_partition,
        })
    }));
    Value::Array(items)
}

fn set_child_webview_bounds(
    context: &mut ChildWebviewCommandContext<'_>,
    args: &Value,
) -> CommandResult {
    let target_label = requested_label(args, "setWebviewBounds")?;
    let bounds = parse_child_bounds(args, "setWebviewBounds")?;
    let entry = context
        .windows
        .child_webviews
        .get_mut(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?;
    entry
        .webview
        .set_bounds(bounds.to_rect())
        .map_err(|error| format!("setWebviewBounds: {error}"))?;
    entry.bounds = bounds;
    entry.auto_resize = false;
    Ok(Value::Null)
}

fn set_child_webview_visible(
    context: &mut ChildWebviewCommandContext<'_>,
    args: &Value,
) -> CommandResult {
    let target_label = requested_label(args, "setWebviewVisible")?;
    let visible = args
        .get("visible")
        .and_then(Value::as_bool)
        .ok_or_else(|| "setWebviewVisible: missing boolean 'visible'".to_string())?;
    let entry = context
        .windows
        .child_webviews
        .get_mut(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?;
    entry
        .webview
        .set_visible(visible)
        .map_err(|error| format!("setWebviewVisible: {error}"))?;
    entry.visible = visible;
    Ok(Value::Null)
}

fn set_child_webview_auto_resize(
    context: &mut ChildWebviewCommandContext<'_>,
    args: &Value,
) -> CommandResult {
    let target_label = requested_label(args, "setWebviewAutoResize")?;
    let enabled = args
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "setWebviewAutoResize: missing boolean 'enabled'".to_string())?;
    let parent_label = context
        .windows
        .child_webviews
        .get(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?
        .parent_label
        .clone();
    let parent = window_for_label(
        &parent_label,
        &context.windows.native_window,
        &context.windows.secondaries,
    )
    .ok_or_else(|| format!("parent window '{parent_label}' not found"))?;
    #[cfg(target_os = "linux")]
    let parent_container = if parent_label == PRIMARY_LABEL {
        &context.windows.primary_webview_container
    } else {
        &context
            .windows
            .secondaries
            .get(&parent_label)
            .expect("parent window existence checked above")
            .webview_container
    };
    let entry = context
        .windows
        .child_webviews
        .get_mut(target_label)
        .expect("child existence checked above");
    entry.auto_resize = enabled;
    update_auto_resize(
        entry,
        ChildWebviewParent {
            window: parent,
            #[cfg(target_os = "linux")]
            container: parent_container,
        },
    );
    Ok(Value::Null)
}

fn navigate_child_webview(context: &ChildWebviewCommandContext<'_>, args: &Value) -> CommandResult {
    let target_label = requested_label(args, "navigateWebview")?;
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| "navigateWebview: missing 'url'".to_string())?;
    let webview = &context
        .windows
        .child_webviews
        .get(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?
        .webview;
    let current = webview
        .url()
        .map_err(|error| format!("navigateWebview: current URL: {error}"))?;
    let resolved = resolve_navigation_url(url, &current)
        .map_err(|error| format!("navigateWebview: {error}"))?;
    webview
        .load_url(&resolved)
        .map_err(|error| format!("navigateWebview: {error}"))?;
    Ok(Value::Null)
}

fn child_webview_url(context: &ChildWebviewCommandContext<'_>, args: &Value) -> CommandResult {
    let target_label = requested_label(args, "getWebviewUrl")?;
    let url = context
        .windows
        .child_webviews
        .get(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?
        .webview
        .url()
        .map_err(|error| format!("getWebviewUrl: {error}"))?;
    Ok(Value::String(url))
}

fn set_child_webview_zoom(
    context: &mut ChildWebviewCommandContext<'_>,
    args: &Value,
) -> CommandResult {
    let target_label = requested_label(args, "setWebviewZoom")?;
    let level = parse_zoom_level(args, "setWebviewZoom")?;
    context
        .windows
        .child_webviews
        .get(target_label)
        .ok_or_else(|| format!("webview '{target_label}' not found"))?
        .webview
        .zoom(level)
        .map_err(|error| format!("setWebviewZoom: {error}"))?;
    context
        .windows
        .zoom_levels
        .insert(target_label.to_string(), level);
    Ok(Value::Null)
}

fn open_child_webview_devtools(
    context: &ChildWebviewCommandContext<'_>,
    args: &Value,
) -> CommandResult {
    #[cfg(debug_assertions)]
    {
        child_webview_operation(
            args,
            "openWebviewDevTools",
            &context.windows.child_webviews,
            |webview| {
                webview.open_devtools();
                Ok(())
            },
        )
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (context, args);
        Err("openWebviewDevTools: only available in debug builds".into())
    }
}

fn close_child_webview_devtools(
    context: &ChildWebviewCommandContext<'_>,
    args: &Value,
) -> CommandResult {
    child_webview_operation(
        args,
        "closeWebviewDevTools",
        &context.windows.child_webviews,
        |webview| {
            #[cfg(debug_assertions)]
            webview.close_devtools();
            let _ = webview;
            Ok(())
        },
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_listed_method_resolves_to_a_distinct_route() {
        let mut names = HashSet::new();
        let mut routes = Vec::new();
        for (name, route) in CHILD_METHODS {
            assert!(names.insert(*name), "{name} listed twice");
            assert!(!routes.contains(route), "{name} shares a route");
            routes.push(*route);
            assert_eq!(parse_method(name), Some(*route), "{name}");
        }
        assert_eq!(CHILD_METHODS.len(), 20);
    }

    #[test]
    fn other_window_methods_are_not_claimed() {
        for method in ["create", "setTitle", "close", "all", "showContextMenu", ""] {
            assert_eq!(parse_method(method), None, "{method}");
        }
    }

    #[test]
    fn only_cookie_methods_skip_the_multiwindow_gate() {
        for (name, route) in CHILD_METHODS {
            assert_eq!(
                route.requires_multiwindow(),
                !name.ends_with("Cookie") && !name.ends_with("Cookies"),
                "{name}"
            );
        }
    }

    #[test]
    fn cookie_routes_do_not_depend_on_the_multiwindow_feature() {
        // Whatever the build, cookie commands must reach their handler (the
        // webview lookup decides what exists), never the api_disabled error.
        for method in [
            "getWebviewCookies",
            "setWebviewCookie",
            "deleteWebviewCookie",
            "clearWebviewCookies",
        ] {
            assert!(!parse_method(method).unwrap().requires_multiwindow());
        }
        assert!(parse_method("createWebview")
            .unwrap()
            .requires_multiwindow());
    }

    #[test]
    fn own_cookies_are_always_reachable() {
        assert!(check_cookie_target("shop", "shop", "get", |_| false).is_ok());
    }

    #[test]
    fn local_callers_may_reach_other_webviews_cookies() {
        assert!(check_cookie_target("main", "shop", "get", |label| label == "main").is_ok());
    }

    #[test]
    fn remote_callers_cannot_reach_other_webviews_cookies() {
        let error =
            check_cookie_target("shop", "main", "getWebviewCookies", |label| label != "shop")
                .unwrap_err();
        assert!(error.starts_with("getWebviewCookies: "), "{error}");
    }

    fn scope(url: Option<&str>, domain: Option<&str>) -> Result<(), String> {
        let url = url.map(|raw| url::Url::parse(raw).unwrap());
        check_remote_cookie_scope(
            "https://shop.example",
            "getWebviewCookies",
            url.as_ref(),
            domain,
        )
    }

    #[test]
    fn remote_callers_must_name_a_url() {
        assert!(scope(None, None).is_err());
    }

    #[test]
    fn remote_callers_are_limited_to_their_own_origin() {
        assert!(scope(Some("https://shop.example/cart"), None).is_ok());
        assert!(scope(Some("http://tynd.localhost/"), None).is_err());
        assert!(scope(Some("https://other.example/"), None).is_err());
        assert!(scope(Some("http://shop.example/"), None).is_err());
        assert!(scope(Some("https://shop.example:8443/"), None).is_err());
    }

    #[test]
    fn remote_cookie_domain_must_stay_inside_the_host() {
        let url = Some("https://shop.example/");
        assert!(scope(url, Some("shop.example")).is_ok());
        assert!(scope(url, Some(".Shop.example")).is_ok());
        assert!(scope(url, Some("api.shop.example")).is_ok());
        assert!(scope(url, Some("tynd.localhost")).is_err());
        assert!(scope(url, Some("example")).is_err());
        assert!(scope(url, Some("evilshop.example")).is_err());
    }
}
