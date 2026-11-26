use serde_json::json;
use tao::window::Window;

/// Last-seen minimize/maximize/fullscreen flags. Used to emit synthetic
/// transition events since tao doesn't surface them directly.
pub(super) struct WindowState {
    minimized: bool,
    maximized: bool,
    fullscreen: bool,
}

impl WindowState {
    pub(super) fn capture(win: &Window) -> Self {
        Self {
            minimized: win.is_minimized(),
            maximized: win.is_maximized(),
            fullscreen: win.fullscreen().is_some(),
        }
    }

    /// Compare current window flags to the last seen; emit events on flips.
    pub(super) fn diff_and_emit(&mut self, win: &Window, label: &str) {
        let next = Self::capture(win);
        if next.minimized != self.minimized {
            crate::ipc::events::emit(
                if next.minimized {
                    "window:minimized"
                } else {
                    "window:unminimized"
                },
                &json!({ "label": label }),
            );
        }
        if next.maximized != self.maximized {
            crate::ipc::events::emit(
                if next.maximized {
                    "window:maximized"
                } else {
                    "window:unmaximized"
                },
                &json!({ "label": label }),
            );
        }
        if next.fullscreen != self.fullscreen {
            crate::ipc::events::emit(
                if next.fullscreen {
                    "window:fullscreen"
                } else {
                    "window:unfullscreen"
                },
                &json!({ "label": label }),
            );
        }
        *self = next;
    }
}
