use serde_json::Value;

use tynd_protocol::runtime::BackendEvent;

#[derive(Debug)]
pub(crate) enum UserEvent {
    Backend(BackendEvent),
    PageReady {
        label: String,
    },
    OsResult {
        /// Label of the window that issued the os_call: the result is
        /// evaluated back on that webview only.
        label: String,
        id: String,
        ok: bool,
        value: Value,
    },
    /// Drain a bounded batch of pre-serialized OS/plugin events. Producers
    /// share a single-flight wakeup so high-rate streams cannot flood tao's
    /// internal event queue.
    FlushOsEvents,
    WindowCmd {
        label: String,
        id: String,
        method: String,
        args: Value,
        /// Origin of the frame that issued the call (`None` = app-local),
        /// captured at IPC time so per-call checks do not depend on later
        /// main-frame tracking.
        origin: Option<String>,
    },
    /// Destroy a child webview after its close RPC response has been
    /// evaluated in the caller. This matters when a child closes itself.
    CloseChildWebview {
        label: String,
    },
    /// Destroy a secondary native window after its close RPC response has
    /// reached the caller.
    CloseSecondaryWindow {
        label: String,
    },
    TrayCmd {
        label: String,
        id: String,
        method: String,
        args: Value,
    },
    /// Menu-bar mutations (`menu.update`, `menu.setItemEnabled`, ...). Routed
    /// through the event loop because muda's items are `!Send` on
    /// Windows / macOS and live on the main thread alongside the menu bar.
    MenuCmd {
        label: String,
        id: String,
        method: String,
        args: Value,
    },
    /// Sent by a timeout thread when on_close takes too long.
    ForceExit,
    /// Sent 500ms after CloseRequested if `cancelClose()` wasn't called.
    ProceedClose,
    /// Sent by the single-instance listener when a duplicate launch forwards
    /// its argv: we bring the primary window to the front.
    #[cfg_attr(not(feature = "api-single-instance"), allow(dead_code))]
    FocusPrimary,
    /// Sent 10ms after the first buffered yield (or synchronously when the
    /// buffer exceeds `YIELD_BATCH_MAX`) to flush pending chunks to every
    /// webview in one `evaluate_script` per window.
    FlushYields,
    /// Web content process / GPU process crashed. Triggers a reload of the
    /// affected webview when `window.contextLostRecovery` is enabled, plus
    /// emits `webview:context-lost` so the frontend can react. macOS / iOS
    /// fire this via `WKWebView`'s `webViewWebContentProcessDidTerminate:`;
    /// on Windows + Linux the underlying compositor recovers transparently
    /// and the host stays silent, hence the platform-conditional `dead_code`
    /// allow.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ContextLost {
        label: String,
    },
    /// Debug protocol asks the host to evaluate JS in a webview and ship the
    /// result back through `reply`. Routed through the event loop because
    /// `wry::WebView` handles aren't `Send` on every platform.
    #[cfg(feature = "debug-protocol")]
    DebugEval {
        label: String,
        script: String,
        reply: crossbeam_channel::Sender<Result<Value, String>>,
    },
    /// Debug protocol asks the host to mutate a window (focus, move,
    /// resize, ...). Routed through the event loop because
    /// `tao::window::Window` handles aren't `Send` on every platform.
    #[cfg(feature = "debug-protocol")]
    DebugWindowManage {
        action: String,
        params: Value,
        reply: crossbeam_channel::Sender<Result<Value, String>>,
    },
}
