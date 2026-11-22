use std::collections::HashMap;

use serde_json::Value;
use tao::{
    event_loop::EventLoopWindowTarget,
    window::{Window, WindowBuilder},
};
use wry::WebViewBuilder;

#[cfg(target_os = "macos")]
use super::util::webview_partition_identifier;
#[cfg(target_os = "linux")]
use super::LinuxWebviewContainer;
use super::{
    child_webview::ChildWebviewEntry,
    util::{validate_data_partition, webview_partition_data_dir},
    SecondaryEntry, UserEvent, WindowState, PRIMARY_LABEL,
};
use crate::webview::{
    builder::{with_debug_tools, with_tynd_runtime},
    handlers::{apply_webview_policy, WebviewPolicy},
    navigation::{check_navigation_target, configure_webview_navigation},
};
use crate::window::{apply_initial_state, validate_window_config, window_builder, WindowConfig};
use tynd_protocol::runtime::BackendCall;
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
fn with_parent_window(wb: WindowBuilder, parent: &Window) -> WindowBuilder {
    use tao::platform::unix::{WindowBuilderExtUnix, WindowExtUnix};
    wb.with_transient_for(parent.gtk_window())
}

#[derive(Debug, Clone)]
pub(super) struct SecondaryWindowRequest {
    pub(super) label: String,
    /// Native-window options, the same set as `app.start({ window })`.
    pub(super) window: WindowConfig,
    pub(super) modal_to: Option<String>,
    pub(super) data_partition: Option<String>,
    pub(super) custom_url: Option<String>,
    pub(super) allow_load_html: bool,
    pub(super) policy: WebviewPolicy,
}

pub(super) fn parse_secondary_window_request(
    args: &Value,
    label_exists: impl Fn(&str) -> bool,
) -> Result<SecondaryWindowRequest, String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if label.is_empty() {
        return Err("create: missing 'label'".into());
    }
    tynd_capabilities::WindowLabel::try_new(label.as_str()).map_err(|_| {
        "create: label must start with a letter and contain at most 64 ASCII letters, \
         digits, '_' or '-'"
            .to_string()
    })?;
    if label == PRIMARY_LABEL {
        return Err(format!("create: label '{PRIMARY_LABEL}' is reserved"));
    }
    if label_exists(&label) {
        return Err(format!("create: window '{label}' already exists"));
    }

    let mut window: WindowConfig = serde_json::from_value(args.clone())
        .map_err(|error| format!("create: invalid window options: {error}"))?;
    validate_window_config(&window).map_err(|error| format!("create: {error}"))?;
    window.title.get_or_insert_with(|| label.clone());

    let custom_url = args.get("url").and_then(Value::as_str).map(str::to_string);
    if let Some(url) = custom_url.as_deref() {
        check_navigation_target(url).map_err(|error| format!("window.create: {error}"))?;
    }
    let data_partition = args
        .get("dataPartition")
        .and_then(Value::as_str)
        .map(|value| validate_data_partition(value).map_err(|error| format!("create: {error}")))
        .transpose()?;
    if window.incognito.unwrap_or(false) && data_partition.is_some() {
        return Err("create: incognito and dataPartition are mutually exclusive".into());
    }

    let policy = WebviewPolicy::from_args(args, "create")?;

    Ok(SecondaryWindowRequest {
        policy,
        window,
        modal_to: args
            .get("modalTo")
            .and_then(Value::as_str)
            .map(str::to_string),
        data_partition,
        allow_load_html: args
            .get("allowLoadHtml")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        label,
        custom_url,
    })
}

fn ensure_partition_available(
    request: &SecondaryWindowRequest,
    secondaries: &HashMap<String, SecondaryEntry>,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
) -> Result<(), String> {
    let Some(partition) = request.data_partition.as_deref() else {
        return Ok(());
    };
    let in_use = secondaries
        .values()
        .any(|entry| entry.data_partition.as_deref() == Some(partition))
        || child_webviews
            .values()
            .any(|entry| entry.data_partition.as_deref() == Some(partition));
    if in_use {
        return Err(format!(
            "create: dataPartition '{partition}' is already used by an open window"
        ));
    }
    Ok(())
}

fn build_secondary_window(
    target: &EventLoopWindowTarget<UserEvent>,
    request: &SecondaryWindowRequest,
    primary: &Window,
    secondaries: &HashMap<String, SecondaryEntry>,
) -> Result<Window, String> {
    let mut builder = window_builder(&request.window, (800, 600));

    // Modal / owned child window: Linux uses GTK transient-for.
    if let Some(parent_label) = request.modal_to.as_deref() {
        let parent = if parent_label == PRIMARY_LABEL {
            primary
        } else {
            secondaries
                .get(parent_label)
                .map(|entry| &entry.window)
                .ok_or_else(|| format!("create: parent window '{parent_label}' not found"))?
        };
        builder = with_parent_window(builder, parent);
    }
    let window = builder
        .build(target)
        .map_err(|error| format!("build window: {error}"))?;
    apply_initial_state(&window, &request.window);
    Ok(window)
}

fn configure_secondary_navigation<'a>(
    builder: WebViewBuilder<'a>,
    request: &SecondaryWindowRequest,
    dev_url: Option<&str>,
    frontend_dir: Option<&str>,
) -> Result<WebViewBuilder<'a>, String> {
    // Custom URLs pass the same off-origin security gate as navigation_handler.
    configure_webview_navigation(
        builder,
        request.custom_url.as_deref(),
        dev_url,
        frontend_dir,
        "window.create",
    )
}
struct SecondaryCreationContext<'a> {
    target: &'a EventLoopWindowTarget<UserEvent>,
    args: &'a Value,
    debug: bool,
    call_tx: &'a crossbeam_channel::Sender<BackendCall>,
    proxy: &'a tao::event_loop::EventLoopProxy<UserEvent>,
    dev_url: Option<&'a str>,
    frontend_dir: Option<&'a str>,
    app_name: &'a str,
    primary: &'a Window,
    secondaries: &'a HashMap<String, SecondaryEntry>,
    child_webviews: &'a HashMap<String, ChildWebviewEntry>,
}

pub(super) type CreateSecondaryFn = for<'a> fn(
    &'a EventLoopWindowTarget<UserEvent>,
    &'a Value,
    bool,
    &'a crossbeam_channel::Sender<BackendCall>,
    &'a tao::event_loop::EventLoopProxy<UserEvent>,
    Option<&'a str>,
    Option<&'a str>,
    &'a str,
    &'a Window,
    &'a HashMap<String, SecondaryEntry>,
    &'a HashMap<String, ChildWebviewEntry>,
) -> Result<(String, SecondaryEntry), String>;

pub(super) const CREATE_SECONDARY: CreateSecondaryFn =
    |target,
     args,
     debug,
     call_tx,
     proxy,
     dev_url,
     frontend_dir,
     app_name,
     primary,
     secondaries,
     child_webviews| {
        create_secondary_with_context(SecondaryCreationContext {
            target,
            args,
            debug,
            call_tx,
            proxy,
            dev_url,
            frontend_dir,
            app_name,
            primary,
            secondaries,
            child_webviews,
        })
    };

/// Create a secondary window + webview + state tracker. Called on the main
/// thread inside the event loop (wry/tao require it).
pub(super) use CREATE_SECONDARY as create_secondary;

struct SecondaryWebviewConfig<'a> {
    debug: bool,
    call_tx: &'a crossbeam_channel::Sender<BackendCall>,
    proxy: &'a tao::event_loop::EventLoopProxy<UserEvent>,
    dev_url: Option<&'a str>,
    frontend_dir: Option<&'a str>,
    #[cfg(target_os = "macos")]
    app_name: &'a str,
}

fn configure_secondary_webview<'a>(
    mut builder: WebViewBuilder<'a>,
    request: &SecondaryWindowRequest,
    label: &str,
    protocol_id: &'a str,
    config: SecondaryWebviewConfig<'_>,
) -> Result<WebViewBuilder<'a>, String> {
    let SecondaryWebviewConfig {
        debug,
        call_tx,
        proxy,
        dev_url,
        frontend_dir,
        #[cfg(target_os = "macos")]
        app_name,
    } = config;

    builder = builder
        .with_id(protocol_id)
        .with_incognito(request.window.incognito.unwrap_or(false))
        .with_transparent(request.window.transparent.unwrap_or(false));
    builder = with_tynd_runtime(builder, label, label, call_tx, proxy);

    if let Some(user_agent) = request.window.user_agent.as_deref() {
        builder = builder.with_user_agent(user_agent);
    }
    builder = apply_webview_policy(
        builder,
        label,
        &request.policy,
        request.custom_url.as_deref(),
    );
    builder = with_debug_tools(builder, debug);

    // Secondary windows inherit the primary's WebGPU / FPS / HDR knobs by
    // default — the GPU process is shared, so per-window divergence
    // wouldn't actually take effect. We still wire the context-lost
    // handler per-window so a recovery reload only touches the affected
    // surface.
    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        if let Some(partition) = request.data_partition.as_deref() {
            builder = builder
                .with_data_store_identifier(webview_partition_identifier(app_name, partition));
        }
        builder =
            crate::webview::builder::with_context_lost_handler(builder, proxy, label.to_owned());
    }

    configure_secondary_navigation(builder, request, dev_url, frontend_dir)
}

fn create_secondary_with_context(
    context: SecondaryCreationContext<'_>,
) -> Result<(String, SecondaryEntry), String> {
    let SecondaryCreationContext {
        target,
        args,
        debug,
        call_tx,
        proxy,
        dev_url,
        frontend_dir,
        app_name,
        primary,
        secondaries,
        child_webviews,
    } = context;
    let request = parse_secondary_window_request(args, |label| {
        secondaries.contains_key(label) || child_webviews.contains_key(label)
    })?;
    let label = request.label.clone();
    let protocol_id = crate::webview::scheme_bin::renderer_id(&label, &label);
    ensure_partition_available(&request, secondaries, child_webviews)?;

    let window = build_secondary_window(target, &request, primary, secondaries)?;
    #[cfg(target_os = "linux")]
    let webview_container = LinuxWebviewContainer::new(&window)
        .map_err(|e| format!("build Linux GTK WebView container: {e}"))?;

    let mut web_context = request.data_partition.as_deref().map(|partition| {
        Box::new(wry::WebContext::new(webview_partition_data_dir(
            app_name, partition,
        )))
    });
    let builder = if let Some(web_context) = web_context.as_deref_mut() {
        WebViewBuilder::new_with_web_context(web_context)
    } else {
        WebViewBuilder::new()
    };
    let wvb = configure_secondary_webview(
        builder,
        &request,
        &label,
        &protocol_id,
        SecondaryWebviewConfig {
            debug,
            call_tx,
            proxy,
            dev_url,
            frontend_dir,
            #[cfg(target_os = "macos")]
            app_name,
        },
    )?;

    #[cfg(not(target_os = "linux"))]
    let webview_result = wvb.build(&window);
    #[cfg(target_os = "linux")]
    let webview_result = webview_container.build_root(wvb);
    let webview = webview_result.map_err(|e| format!("build webview: {e}"))?;
    crate::webview::process_failed::watch(&webview, &label);
    if request.window.visible.unwrap_or(true) {
        window.set_visible(true);
    }
    let state = WindowState::capture(&window);
    Ok((
        label,
        SecondaryEntry {
            webview,
            _web_context: web_context,
            #[cfg(target_os = "linux")]
            webview_container,
            menu: crate::api::menu::WindowMenu::default(),
            window,
            data_partition: request.data_partition,
            state,
            allow_load_html: request.allow_load_html,
        },
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(args: &Value) -> Result<SecondaryWindowRequest, String> {
        parse_secondary_window_request(args, |label| label == "taken")
    }

    #[test]
    fn accepts_the_main_window_option_set() {
        let request = parse(&json!({
            "label": "settings",
            "x": 10, "y": -20, "minWidth": 300, "minHeight": 200,
            "visible": false, "transparent": true, "fullscreen": false,
            "maximized": true, "center": true, "theme": "dark", "skipTaskbar": true,
            "backgroundColor": { "r": 0, "g": 0, "b": 0, "a": 0 },
            "kiosk": false, "titleBarStyle": "overlay", "alwaysOnTop": true,
            "modalTo": "main", "hideOnEscape": true
        }))
        .unwrap();
        let window = &request.window;
        assert_eq!(window.title.as_deref(), Some("settings"));
        assert_eq!((window.x, window.y), (Some(10.0), Some(-20.0)));
        assert_eq!(window.visible, Some(false));
        assert_eq!(window.skip_taskbar, Some(true));
        assert_eq!(request.modal_to.as_deref(), Some("main"));
    }

    #[test]
    fn rejects_invalid_labels_and_options() {
        for (args, needle) in [
            (json!({}), "missing 'label'"),
            (json!({ "label": "main" }), "reserved"),
            (json!({ "label": "taken" }), "already exists"),
            (
                json!({ "label": "1bad" }),
                "64 ASCII letters, digits, '_' or '-'",
            ),
            (json!({ "label": "w", "width": 0 }), "'width'"),
            (
                json!({ "label": "w", "width": -5 }),
                "invalid window options",
            ),
            (
                json!({ "label": "w", "theme": "sepia" }),
                "invalid window options",
            ),
            (
                json!({ "label": "w", "incognito": true, "dataPartition": "a" }),
                "mutually exclusive",
            ),
        ] {
            let error = parse(&args).unwrap_err();
            assert!(error.contains(needle), "{args}: {error}");
        }
    }
}
