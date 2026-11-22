use crossbeam_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::window::WindowConfig;

/// Terminal payload of a backend call.
#[derive(Debug, Clone)]
pub enum BackendReturnPayload {
    Ok(String),
    Err(String),
}

impl BackendReturnPayload {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }
}

/// A message from the backend runtime to the frontend.
#[derive(Debug)]
pub enum BackendEvent {
    Return {
        id: String,
        payload: BackendReturnPayload,
    },
    Yield {
        id: String,
        value: String,
    },
    Emit {
        name: String,
        payload: String,
        to: Option<String>,
    },
    EvalInFrontend {
        script: String,
        to: Option<String>,
    },
    Reload,
    Error {
        message: String,
    },
}

/// Everything `run_app` needs to talk to the backend.
#[derive(Debug)]
pub struct BackendBridge {
    pub config: BackendConfig,
    pub call_tx: Sender<BackendCall>,
    pub event_rx: Receiver<BackendEvent>,
}

/// Reserved `fn_name` of the typed call carrying a window lifecycle event.
pub const WINDOW_EVENT_FN: &str = "__tynd_window_event__";

/// Window lifecycle transition reported to the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowEventKind {
    Created,
    Closed,
}

impl WindowEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Closed => "closed",
        }
    }
}

/// A call or lifecycle signal sent to the backend runtime.
#[derive(Debug)]
pub enum BackendCall {
    Raw(String),
    Typed {
        id: String,
        fn_name: String,
        args: Vec<Value>,
    },
    Cancel {
        id: String,
    },
    Ack {
        id: String,
        n: u32,
    },
    /// Result of a trusted Bun-backend call into the native host.
    HostResult {
        id: String,
        ok: bool,
        value: Value,
    },
    /// A native window or webview event (`window:resized`, `webview:page-load`…)
    /// delivered to the backend so `getWindow(label).on*` handlers run there.
    /// The Bun runtime receives it as `{ "type": "tynd:os_event", name, data }`.
    OsEvent {
        name: String,
        data: Value,
    },
    /// A webview request to a backend-served custom protocol
    /// (`app.protocol(...)`). Answered by the runtime through
    /// `tynd_host::resolve_backend_protocol`.
    Protocol(Box<ProtocolRequest>),
}

/// One webview request forwarded to a backend protocol handler. The body is
/// raw bytes here; the runtime picks the wire encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolRequest {
    pub id: String,
    pub scheme: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Label of the webview that issued the request.
    pub webview: String,
}

/// Opt-in `asset://` protocol: absolute-path globs a webview may read through
/// `convertFileSrc`. Absent means the protocol is not registered.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssetsConfig {
    pub scope: Vec<String>,
}

/// Whether the host forwards OS event `name` to the backend: window and
/// webview events only, minus the per-pixel `window:drag-over` stream.
pub fn forwards_os_event(name: &str) -> bool {
    (name.starts_with("window:") || name.starts_with("webview:")) && name != "window:drag-over"
}

impl BackendCall {
    pub fn call(id: impl Into<String>, fn_name: impl Into<String>, args: Vec<Value>) -> Self {
        Self::Typed {
            id: id.into(),
            fn_name: fn_name.into(),
            args,
        }
    }

    /// Notify the backend that a window was created (`"created"`) or
    /// destroyed (`"closed"`). The Bun runtime forwards it as the
    /// `{ "type": "tynd:window", "event", "label" }` frame.
    pub fn window_event(event: WindowEventKind, label: impl Into<String>) -> Self {
        Self::Typed {
            id: WINDOW_EVENT_FN.to_string(),
            fn_name: WINDOW_EVENT_FN.to_string(),
            args: vec![
                Value::String(event.as_str().to_string()),
                Value::String(label.into()),
            ],
        }
    }

    pub fn lifecycle(fn_name: &str) -> Self {
        let id = format!("__tynd_{fn_name}__");
        let fn_name = format!("__tynd_{fn_name}__");
        Self::Typed {
            id,
            fn_name,
            args: vec![],
        }
    }
}

/// Config provided by the backend at startup.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BackendConfig {
    pub window: WindowConfig,
    /// Host policy supplied by the trusted backend before WebView creation.
    pub security: Option<Value>,
    /// Explicit renderer grants supplied by the trusted backend.
    /// Missing or empty capabilities keep the native command gate closed.
    pub capabilities: Option<Value>,
    pub dev_url: Option<String>,
    pub frontend_dir: Option<String>,
    pub icon_path: Option<String>,
    pub menu: Vec<MenuItemDef>,
    pub tray: Option<TrayConfig>,
    pub quit_on_last_window_closed: bool,
    /// `asset://` protocol scope; `None` keeps the protocol unregistered.
    pub assets: Option<AssetsConfig>,
    /// Schemes served by backend `app.protocol(...)` handlers, registered on
    /// every webview before it is created.
    pub protocols: Vec<String>,
    #[serde(skip)]
    pub smoke_mode: bool,
}

/// Native OS roles for menu items.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    Type,
    strum::Display,
    strum::EnumString,
    strum::AsRefStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum MenuRole {
    Quit,
    Copy,
    Cut,
    Paste,
    Undo,
    Redo,
    SelectAll,
    Minimize,
    Close,
    Separator,
    About,
    Hide,
    HideOthers,
    ShowAll,
    Services,
    Maximize,
    Fullscreen,
    BringAllToFront,
}

/// Image shown next to a menu item. A plain string is a file path (gated by
/// the fs allow-list); `png` / `rgba` carry base64-encoded pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum MenuIcon {
    Path(String),
    Rgba {
        rgba: String,
        width: u32,
        height: u32,
    },
    Png {
        png: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemDef {
    pub id: Option<String>,
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub role: Option<MenuRole>,
    pub accelerator: Option<String>,
    pub icon: Option<MenuIcon>,
    /// Platform icon name (`"add"`, `"folder"`, …), see `NativeIcon` in muda.
    pub native_icon: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CheckboxDef {
    pub id: Option<String>,
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub accelerator: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RadioDef {
    pub id: Option<String>,
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    pub accelerator: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SubmenuDef {
    pub id: Option<String>,
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub icon: Option<MenuIcon>,
    pub native_icon: Option<String>,
    #[serde(default)]
    pub items: Vec<MenuItemDef>,
}

/// One menu entry, tagged by `type`. A missing `type` means `"submenu"` when
/// the entry has `items`, `"item"` otherwise (same default as the TS clients).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MenuItemDef {
    Separator,
    Submenu(SubmenuDef),
    Checkbox(CheckboxDef),
    Radio(RadioDef),
    Item(ItemDef),
}

/// Parse twin of [`MenuItemDef`]; its `type` tag is filled in beforehand.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum MenuItemWire {
    Separator,
    Submenu(SubmenuDef),
    Checkbox(CheckboxDef),
    Radio(RadioDef),
    Item(ItemDef),
}

impl<'de> Deserialize<'de> for MenuItemDef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut value = Value::deserialize(deserializer)?;
        if let Value::Object(map) = &mut value {
            if !map.contains_key("type") {
                let kind = if map.contains_key("items") {
                    "submenu"
                } else {
                    "item"
                };
                map.insert("type".into(), Value::from(kind));
            }
        }
        Ok(
            match MenuItemWire::deserialize(value).map_err(serde::de::Error::custom)? {
                MenuItemWire::Separator => Self::Separator,
                MenuItemWire::Submenu(submenu) => Self::Submenu(submenu),
                MenuItemWire::Checkbox(checkbox) => Self::Checkbox(checkbox),
                MenuItemWire::Radio(radio) => Self::Radio(radio),
                MenuItemWire::Item(item) => Self::Item(item),
            },
        )
    }
}

impl MenuItemDef {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Separator => None,
            Self::Submenu(s) => s.id.as_deref(),
            Self::Checkbox(c) => c.id.as_deref(),
            Self::Radio(r) => r.id.as_deref(),
            Self::Item(i) => i.id.as_deref(),
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            Self::Separator => None,
            Self::Submenu(s) => s.label.as_deref(),
            Self::Checkbox(c) => c.label.as_deref(),
            Self::Radio(r) => r.label.as_deref(),
            Self::Item(i) => i.label.as_deref(),
        }
    }
}

/// System tray configuration.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct TrayConfig {
    pub icon: String,
    pub tooltip: Option<String>,
    pub menu: Option<Vec<MenuItemDef>>,
}
