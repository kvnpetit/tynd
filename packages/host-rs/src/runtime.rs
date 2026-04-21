use serde::Deserialize;
use serde_json::Value;
use std::sync::mpsc::{Receiver, Sender};

use crate::window::WindowConfig;

/// A message coming from the backend runtime to the frontend.
#[derive(Debug)]
pub enum BackendEvent {
    /// Result of a frontend function call (resolve/reject). For streaming calls,
    /// marks the end of the stream; `value` is the generator's return value (or Null).
    Return { id: String, ok: bool, value: Value },
    /// A single chunk from a streaming (async-iterable) backend call.
    Yield { id: String, value: Value },
    /// Event pushed by the backend to frontend subscribers
    Emit {
        name: String,
        payload: Value,
        /// Target window label; when `None` every webview receives the event.
        to: Option<String>,
    },
    /// Dev-mode: backend was hot-reloaded — the host should soft-reload the webview
    /// so the frontend picks up the new backend without losing the native window.
    Reload,
    /// Dev-mode: backend threw during startup/reload — show an overlay in the webview.
    Error { message: String },
}

/// Everything `run_app` needs to talk to the backend.
/// Constructed by `tynd-full` (Bun bridge) or `tynd-lite` (QuickJS bridge).
#[derive(Debug)]
pub struct BackendBridge {
    /// Window and frontend config sent by the backend at startup
    pub config: BackendConfig,
    /// Send a call (or lifecycle message) to the backend.
    /// Unbounded so the WebView IPC handler never blocks waiting for the backend.
    pub call_tx: Sender<BackendCall>,
    /// Receive results and events from the backend
    pub event_rx: Receiver<BackendEvent>,
}

/// A call or lifecycle signal sent to the backend runtime.
#[derive(Debug)]
pub enum BackendCall {
    /// Raw JSON string from the frontend — forwarded directly to Bun without
    /// re-serialization (full mode only). Skips one full JSON parse+serialize cycle.
    Raw(String),
    /// Parsed call for QuickJS (lite mode) or a lifecycle signal (both modes).
    Typed {
        id: String,
        fn_name: String,
        args: Vec<Value>,
    },
    /// Cancel an in-flight streaming call. Triggers `iterator.return()` on the
    /// backend-side async generator and unblocks the frontend iterator.
    Cancel { id: String },
    /// Credit replenishment from the frontend — the iterator has consumed `n`
    /// chunks. Backend increments its per-stream credit and wakes any yield
    /// awaiting on an empty credit pool. Keeps in-flight memory bounded.
    Ack { id: String, n: u32 },
}

impl BackendCall {
    pub fn call(id: impl Into<String>, fn_name: impl Into<String>, args: Vec<Value>) -> Self {
        Self::Typed {
            id: id.into(),
            fn_name: fn_name.into(),
            args,
        }
    }

    /// Internal lifecycle signal — result is silently discarded by the host.
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
#[derive(Debug, Default, Clone)]
pub struct BackendConfig {
    pub window: WindowConfig,
    /// Overrides frontend_dir if set (dev server)
    pub dev_url: Option<String>,
    /// Directory of built frontend static files
    pub frontend_dir: Option<String>,
    /// Path to the app icon (PNG or ICO). Used for window title bar + taskbar icon.
    pub icon_path: Option<String>,
    /// Native menu bar definition (empty = no menu bar)
    pub menu: Vec<MenuItemDef>,
    /// System tray configuration
    pub tray: Option<TrayConfig>,
    /// When true, exit as soon as the last open window closes (tray-only
    /// apps leave this off so closing every window keeps the tray alive).
    pub quit_on_last_window_closed: bool,
}

/// A single menu item — separator, action, submenu, or native role.
/// Flat struct covers all variants (missing fields are `None`).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct MenuItemDef {
    /// "separator" | "submenu" | "checkbox" | "radio" | "item"
    /// (omitted -> action item)
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// User-defined ID — emitted as `{ id }` on click
    pub id: Option<String>,
    /// Display label
    pub label: Option<String>,
    /// Whether this item is interactable (defaults to true)
    pub enabled: Option<bool>,
    /// Native OS role: "quit" | "copy" | "cut" | "paste" | "undo" | "redo" |
    /// "selectAll" | "minimize" | "close" | "separator"
    pub role: Option<String>,
    /// Keyboard accelerator (muda format — e.g. "CmdOrCtrl+S", "Alt+F4").
    pub accelerator: Option<String>,
    /// Initial check state for checkbox / radio items.
    pub checked: Option<bool>,
    /// Child items for submenus
    pub items: Option<Vec<MenuItemDef>>,
}

/// System tray configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TrayConfig {
    /// Path to tray icon (PNG/ICO/BMP recommended)
    pub icon: String,
    /// Tooltip shown on hover
    pub tooltip: Option<String>,
    /// Context menu shown on tray right-click
    pub menu: Option<Vec<MenuItemDef>>,
}
