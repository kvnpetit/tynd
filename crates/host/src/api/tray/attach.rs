//! Tray-attached windows ("menubar apps"): a left click on the icon toggles
//! the window next to it, and the window hides again when it loses focus.

use std::{
    cell::RefCell,
    collections::HashMap,
    time::{Duration, Instant},
};

use serde_json::Value;
use tao::{dpi::PhysicalPosition, window::Window};

use super::geometry::{position_near_tray, Bounds};

/// Gap between the window and the taskbar / menu bar, in physical pixels.
const DEFAULT_GAP: f64 = 8.0;

/// A click on the icon steals focus from the open window first, which hides
/// it on blur; the click that follows must then not reopen it.
const BLUR_REOPEN_GUARD: Duration = Duration::from_millis(300);

/// Window toggled by a tray icon.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Attachment {
    pub(crate) label: String,
    pub(crate) hide_on_blur: bool,
    pub(crate) gap: f64,
}

/// `attachWindow` support: tray-icon's Linux backend (libappindicator) never
/// reports icon clicks, so an attached window, hidden on attach, could never
/// be shown again.
pub(crate) fn check_supported() -> Result<(), String> {
    if cfg!(target_os = "linux") {
        Err("tray.attachWindow: not supported on linux".into())
    } else {
        Ok(())
    }
}

impl Attachment {
    /// Parse `attachWindow({ label, hideOnBlur?, gap? })`.
    pub(crate) fn parse(args: &Value) -> Result<Self, String> {
        let label = args
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
            .ok_or_else(|| {
                "tray.attachWindow: missing 'label' (the label of an open window, e.g. \"main\")"
                    .to_string()
            })?
            .to_string();
        let hide_on_blur = match args.get("hideOnBlur") {
            None | Some(Value::Null) => true,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| "tray.attachWindow: 'hideOnBlur' must be a boolean".to_string())?,
        };
        Ok(Self {
            label,
            hide_on_blur,
            gap: parse_gap(args, "attachWindow")?,
        })
    }
}

/// Optional `gap` (pixels between the window and the bar), 0..=200.
pub(crate) fn parse_gap(args: &Value, op: &str) -> Result<f64, String> {
    match args.get("gap") {
        None | Some(Value::Null) => Ok(DEFAULT_GAP),
        Some(value) => value
            .as_f64()
            .filter(|gap| (0.0..=200.0).contains(gap))
            .ok_or_else(|| format!("tray.{op}: 'gap' must be a number between 0 and 200")),
    }
}

thread_local! {
    /// When each attached window was last hidden on blur (main thread).
    static BLUR_HIDDEN: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
}

/// What a blur does to an attached window: hide it, and remember the hide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlurOutcome {
    hide: bool,
    record: bool,
}

/// The window may already be hidden when the blur reaches the tray (the
/// generic `hideOnFocusLost` runs first): that hide is recorded too, or the
/// tray click that caused the blur would reopen it.
fn blur_outcome(hide_on_blur: bool, visible: bool) -> BlurOutcome {
    let hide = hide_on_blur && visible;
    BlurOutcome {
        hide,
        record: hide || !visible,
    }
}

/// Hide `window` if `attachment` asks for it; remembers when it got hidden,
/// so the tray click that caused the blur does not reopen it.
pub(crate) fn hide_on_blur(attachment: &Attachment, window: &Window) {
    let outcome = blur_outcome(attachment.hide_on_blur, window.is_visible());
    if outcome.hide {
        window.set_visible(false);
    }
    if outcome.record {
        let _ = BLUR_HIDDEN.try_with(|hidden| {
            if let Ok(mut hidden) = hidden.try_borrow_mut() {
                hidden.insert(attachment.label.clone(), Instant::now());
            }
        });
    }
}

fn hidden_by_blur_just_now(label: &str) -> bool {
    BLUR_HIDDEN
        .try_with(|hidden| {
            hidden
                .try_borrow_mut()
                .ok()
                .and_then(|mut hidden| hidden.remove(label))
                .is_some_and(|at| at.elapsed() < BLUR_REOPEN_GUARD)
        })
        .unwrap_or(false)
}

/// Left click on the icon: hide the window when it is open, otherwise move it
/// next to `anchor` (the icon bounds, or the click point) and show it.
pub(crate) fn toggle(attachment: &Attachment, window: &Window, anchor: &Bounds) {
    if window.is_visible() {
        window.set_visible(false);
        return;
    }
    if hidden_by_blur_just_now(&attachment.label) {
        return;
    }
    place(window, anchor, attachment.gap);
    window.set_visible(true);
    window.set_focus();
}

/// Move `window` next to `anchor`, inside the work area of the monitor that
/// holds it.
pub(crate) fn place(window: &Window, anchor: &Bounds, gap: f64) {
    let (cx, cy) = (
        anchor.x + anchor.width / 2.0,
        anchor.y + anchor.height / 2.0,
    );
    let handle = window
        .available_monitors()
        .find(|monitor| monitor_bounds(monitor).contains(cx, cy))
        .or_else(|| window.primary_monitor());
    let Some(handle) = handle else {
        return;
    };
    let monitor = monitor_bounds(&handle);
    let work_area = work_area(&handle).unwrap_or(monitor);
    let size = window.outer_size();
    let (x, y) = position_near_tray(
        anchor,
        f64::from(size.width),
        f64::from(size.height),
        &monitor,
        &work_area,
        gap,
    );
    window.set_outer_position(PhysicalPosition::new(x.round(), y.round()));
}

fn monitor_bounds(monitor: &tao::monitor::MonitorHandle) -> Bounds {
    let position = monitor.position();
    let size = monitor.size();
    Bounds {
        x: f64::from(position.x),
        y: f64::from(position.y),
        width: f64::from(size.width),
        height: f64::from(size.height),
    }
}

/// Work area of `monitor` (minus taskbar, menu bar and Dock, or panels):
/// `GetMonitorInfo` on Windows, `NSScreen.visibleFrame` on macOS, the GDK
/// work area on Linux.
fn work_area(monitor: &tao::monitor::MonitorHandle) -> Option<Bounds> {
    crate::api::window_cmd::monitor_work_area(monitor).map(|area| Bounds {
        x: f64::from(area.x),
        y: f64::from(area.y),
        width: f64::from(area.width),
        height: f64::from(area.height),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn attachment_parse_defaults_and_validation() {
        let attachment = Attachment::parse(&json!({ "label": "popup" })).unwrap();
        assert_eq!(
            attachment,
            Attachment {
                label: "popup".into(),
                hide_on_blur: true,
                gap: DEFAULT_GAP,
            }
        );
        let custom =
            Attachment::parse(&json!({ "label": "main", "hideOnBlur": false, "gap": 0 })).unwrap();
        assert!(!custom.hide_on_blur);
        assert!(custom.gap.abs() < f64::EPSILON);

        for bad in [
            json!({}),
            json!({ "label": "" }),
            json!({ "label": "p", "hideOnBlur": "yes" }),
            json!({ "label": "p", "gap": -1 }),
            json!({ "label": "p", "gap": 5000 }),
        ] {
            assert!(Attachment::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn blur_records_hides_done_before_the_tray_hook() {
        // Visible: hidden here when asked to, and recorded.
        assert_eq!(
            blur_outcome(true, true),
            BlurOutcome {
                hide: true,
                record: true
            }
        );
        assert_eq!(
            blur_outcome(false, true),
            BlurOutcome {
                hide: false,
                record: false
            }
        );
        // Already hidden by `hideOnFocusLost`: still recorded.
        for hide_on_blur in [true, false] {
            assert_eq!(
                blur_outcome(hide_on_blur, false),
                BlurOutcome {
                    hide: false,
                    record: true
                }
            );
        }
    }

    #[test]
    fn attach_window_is_rejected_on_linux_only() {
        let result = check_supported();
        if cfg!(target_os = "linux") {
            assert_eq!(
                result.unwrap_err(),
                "tray.attachWindow: not supported on linux"
            );
        } else {
            assert!(result.is_ok());
        }
    }

    #[test]
    fn blur_guard_is_consumed_once() {
        BLUR_HIDDEN.with(|hidden| {
            hidden.borrow_mut().insert("popup".into(), Instant::now());
        });
        assert!(hidden_by_blur_just_now("popup"));
        assert!(!hidden_by_blur_just_now("popup"));
        assert!(!hidden_by_blur_just_now("other"));
    }
}
