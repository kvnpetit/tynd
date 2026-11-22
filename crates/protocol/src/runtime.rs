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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ItemDef {
    pub id: Option<String>,
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub role: Option<MenuRole>,
    pub accelerator: Option<String>,
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
    #[serde(default)]
    pub items: Vec<MenuItemDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MenuItemDef {
    Separator,
    Submenu(SubmenuDef),
    Checkbox(CheckboxDef),
    Radio(RadioDef),
    Item(ItemDef),
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
