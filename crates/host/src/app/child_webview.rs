use crossbeam_channel::Sender;
use serde_json::Value;
use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    Rect, WebContext, WebView, WebViewBuilder,
};

#[cfg(target_os = "linux")]
use super::linux_webview::LinuxWebviewContainer;
use super::{
    util::{validate_data_partition, webview_partition_data_dir},
    UserEvent,
};
use crate::webview::{
    builder::{with_debug_tools, with_tynd_runtime},
    handlers::{apply_webview_policy, WebviewPolicy},
    navigation::{self, check_navigation_target, configure_webview_navigation},
};
use tynd_protocol::runtime::BackendCall;

const MAX_CHILD_WEBVIEW_DIMENSION: f64 = 32_768.0;

pub(super) struct ChildWebviewParent<'a> {
    pub(super) window: &'a Window,
    #[cfg(target_os = "linux")]
    pub(super) container: &'a LinuxWebviewContainer,
}

impl ChildWebviewParent<'_> {
    fn logical_size(&self) -> LogicalSize<f64> {
        #[cfg(target_os = "linux")]
        {
            self.container.logical_size(self.window)
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.window
                .inner_size()
                .to_logical::<f64>(self.window.scale_factor())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ChildWebviewDisplay {
    pub(super) visible: bool,
    pub(super) focused: bool,
    pub(super) auto_resize: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ChildWebviewRequest {
    pub(super) label: String,
    pub(super) parent_label: String,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) display: ChildWebviewDisplay,
    pub(super) user_agent: Option<String>,
    pub(super) incognito: bool,
    pub(super) data_partition: Option<String>,
    pub(super) custom_url: Option<String>,
    pub(super) policy: WebviewPolicy,
}

impl ChildWebviewRequest {
    /// Install the new-window, download, proxy and plugin hooks for this webview.
    fn apply_policy<'a>(&self, builder: WebViewBuilder<'a>) -> WebViewBuilder<'a> {
        apply_webview_policy(
            builder,
            &self.label,
            &self.policy,
            self.custom_url.as_deref(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ChildBounds {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

impl ChildBounds {
    pub(super) fn to_rect(self) -> Rect {
        Rect {
            position: LogicalPosition::new(self.x, self.y).into(),
            size: LogicalSize::new(self.width, self.height).into(),
        }
    }
}

pub(super) fn parse_child_bounds(args: &Value, operation: &str) -> Result<ChildBounds, String> {
    let number = |key: &str| -> Result<f64, String> {
        let value = args
            .get(key)
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("{operation}: missing numeric '{key}'"))?;
        if !value.is_finite() {
            return Err(format!("{operation}: '{key}' must be finite"));
        }
        Ok(value)
    };
    let bounds = ChildBounds {
        x: number("x")?,
        y: number("y")?,
        width: number("width")?,
        height: number("height")?,
    };
    if !(0.0 < bounds.width
        && bounds.width <= MAX_CHILD_WEBVIEW_DIMENSION
        && 0.0 < bounds.height
        && bounds.height <= MAX_CHILD_WEBVIEW_DIMENSION)
    {
        return Err(format!(
            "{operation}: width and height must be within 0..={MAX_CHILD_WEBVIEW_DIMENSION}"
        ));
    }
    Ok(bounds)
}

pub(super) struct ChildWebviewEntry {
    pub(super) webview: WebView,
    // Must outlive `webview`; field order intentionally drops WebView first.
    pub(super) _web_context: Option<Box<WebContext>>,
    pub(super) parent_label: String,
    pub(super) data_partition: Option<String>,
    pub(super) bounds: ChildBounds,
    pub(super) visible: bool,
    pub(super) auto_resize: bool,
}

pub(super) fn parse_child_webview_request(
    args: &Value,
    source_window_label: &str,
) -> Result<ChildWebviewRequest, String> {
    let label = args
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    tynd_capabilities::WindowLabel::try_new(label.as_str()).map_err(|_| {
        "createWebview: label must start with a letter and contain at most 64 ASCII letters, \
         digits, '_' or '-'"
            .to_string()
    })?;
    if label == super::PRIMARY_LABEL {
        return Err(format!(
            "createWebview: label '{}' is reserved",
            super::PRIMARY_LABEL
        ));
    }
    let parent_label = args
        .get("parent")
        .and_then(Value::as_str)
        .unwrap_or(source_window_label)
        .trim()
        .to_string();
    if parent_label.is_empty() {
        return Err("createWebview: parent must not be empty".into());
    }

    let number = |key: &str, default: Option<f64>| -> Result<f64, String> {
        let value = args
            .get(key)
            .and_then(Value::as_f64)
            .or(default)
            .ok_or_else(|| format!("createWebview: missing numeric '{key}'"))?;
        if !value.is_finite() {
            return Err(format!("createWebview: '{key}' must be finite"));
        }
        Ok(value)
    };
    let x = number("x", Some(0.0))?;
    let y = number("y", Some(0.0))?;
    let width = number("width", None)?;
    let height = number("height", None)?;
    if !(0.0 < width
        && width <= MAX_CHILD_WEBVIEW_DIMENSION
        && 0.0 < height
        && height <= MAX_CHILD_WEBVIEW_DIMENSION)
    {
        return Err(format!(
            "createWebview: width and height must be within 0..={MAX_CHILD_WEBVIEW_DIMENSION}"
        ));
    }

    let custom_url = args.get("url").and_then(Value::as_str).map(str::to_string);
    if let Some(url) = custom_url.as_deref() {
        check_navigation_target(url).map_err(|error| format!("createWebview: {error}"))?;
    }
    let incognito = args
        .get("incognito")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let data_partition = args
        .get("dataPartition")
        .and_then(Value::as_str)
        .map(|value| {
            validate_data_partition(value).map_err(|error| format!("createWebview: {error}"))
        })
        .transpose()?;
    if incognito && data_partition.is_some() {
        return Err("createWebview: incognito and dataPartition are mutually exclusive".into());
    }

    let policy = WebviewPolicy::from_args(args, "createWebview")?;

    Ok(ChildWebviewRequest {
        policy,
        label,
        parent_label,
        x,
        y,
        width,
        height,
        display: ChildWebviewDisplay {
            visible: args.get("visible").and_then(Value::as_bool).unwrap_or(true),
            focused: args
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            auto_resize: args
                .get("autoResize")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        },
        user_agent: args
            .get("userAgent")
            .and_then(Value::as_str)
            .map(str::to_string),
        incognito,
        data_partition,
        custom_url,
    })
}

fn resolve_child_bounds(
    request: &mut ChildWebviewRequest,
    parent: &ChildWebviewParent<'_>,
) -> ChildBounds {
    if request.display.auto_resize {
        let size = parent.logical_size();
        request.x = 0.0;
        request.y = 0.0;
        request.width = size.width;
        request.height = size.height;
    }
    ChildBounds {
        x: request.x,
        y: request.y,
        width: request.width,
        height: request.height,
    }
}
pub(super) fn build_child_webview(
    mut request: ChildWebviewRequest,
    parent: ChildWebviewParent<'_>,
    debug: bool,
    call_tx: &Sender<BackendCall>,
    proxy: &EventLoopProxy<UserEvent>,
    dev_url: Option<&str>,
    frontend_dir: Option<&str>,
    app_name: &str,
) -> Result<(String, ChildWebviewEntry), String> {
    let bounds = resolve_child_bounds(&mut request, &parent);
    let protocol_id =
        crate::webview::scheme_bin::renderer_id(&request.parent_label, &request.label);

    let mut web_context = request.data_partition.as_deref().map(|partition| {
        Box::new(WebContext::new(webview_partition_data_dir(
            app_name, partition,
        )))
    });
    let builder = if let Some(context) = web_context.as_deref_mut() {
        WebViewBuilder::new_with_web_context(context)
    } else {
        WebViewBuilder::new()
    };

    let builder = builder
        .with_id(&protocol_id)
        .with_bounds(bounds.to_rect())
        .with_visible(request.display.visible)
        .with_incognito(request.incognito);
    let mut builder = with_tynd_runtime(
        builder,
        &request.parent_label,
        &request.label,
        call_tx,
        proxy,
    );

    if let Some(user_agent) = request.user_agent.as_deref() {
        builder = builder.with_user_agent(user_agent);
    }
    builder = request.apply_policy(builder);
    builder = with_debug_tools(builder, debug);

    #[cfg(target_os = "macos")]
    {
        use wry::WebViewBuilderExtDarwin;
        if let Some(partition) = request.data_partition.as_deref() {
            builder = builder.with_data_store_identifier(
                super::util::webview_partition_identifier(app_name, partition),
            );
        }
        builder = crate::webview::builder::with_context_lost_handler(
            builder,
            proxy,
            request.label.clone(),
        );
    }

    builder = configure_child_navigation(builder, &request, dev_url, frontend_dir)?;

    #[cfg(not(target_os = "linux"))]
    let webview_result = builder.build_as_child(parent.window);
    #[cfg(target_os = "linux")]
    let webview_result = parent.container.build_child(builder);
    let webview =
        webview_result.map_err(|error| format!("createWebview: build child webview: {error}"))?;
    navigation::register_webview_window(&request.label, &request.parent_label);
    if request.display.focused {
        webview
            .focus()
            .map_err(|error| format!("createWebview: focus: {error}"))?;
    }

    Ok((
        request.label,
        ChildWebviewEntry {
            webview,
            _web_context: web_context,
            parent_label: request.parent_label,
            data_partition: request.data_partition,
            bounds,
            visible: request.display.visible,
            auto_resize: request.display.auto_resize,
        },
    ))
}

fn configure_child_navigation<'a>(
    builder: WebViewBuilder<'a>,
    request: &ChildWebviewRequest,
    dev_url: Option<&str>,
    frontend_dir: Option<&str>,
) -> Result<WebViewBuilder<'a>, String> {
    configure_webview_navigation(
        builder,
        request.custom_url.as_deref(),
        dev_url,
        frontend_dir,
        "createWebview",
    )
}
pub(super) fn update_auto_resize(entry: &mut ChildWebviewEntry, parent: ChildWebviewParent<'_>) {
    if !entry.auto_resize {
        return;
    }
    let size = parent.logical_size();
    entry.bounds = ChildBounds {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    };
    if let Err(error) = entry.webview.set_bounds(entry.bounds.to_rect()) {
        crate::tynd_warn!(
            "child webview auto-resize failed label_parent={:?}: {error}",
            entry.parent_label
        );
    }
}
