//! Monitor enumeration behind the client's `monitors.*`: bounds, work area
//! (the part not covered by the taskbar / menu bar / panels) and scale of
//! every display, plus `monitor:change` events when displays are added,
//! removed or reconfigured (resolution, scale factor, work area).
//!
//! Change detection is lazy: nothing watches the displays until a frontend
//! calls `window.subscribeMonitorChanges`. Each OS source (see `watch`) only
//! says "something changed"; the event loop then re-reads the monitor list
//! and [`diff`] decides whether an event is due, so duplicate or no-op
//! notifications never reach the frontend.

use parking_lot::Mutex;
use serde_json::{json, Value};
use tao::{monitor::MonitorHandle, window::Window};

use super::DispatchResult;

mod watch;

/// Axis-aligned rectangle in physical virtual-desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Rect {
    fn to_json(self) -> Value {
        json!({ "x": self.x, "y": self.y, "width": self.width, "height": self.height })
    }
}

/// One display as reported to the frontend.
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    /// Stable per-session identifier (see [`monitor_id`]).
    id: String,
    name: Option<String>,
    bounds: Rect,
    work_area: Rect,
    scale: f64,
    is_primary: bool,
}

impl Snapshot {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "position": { "x": self.bounds.x, "y": self.bounds.y },
            "size": { "width": self.bounds.width, "height": self.bounds.height },
            "workArea": self.work_area.to_json(),
            "scale": self.scale,
            "isPrimary": self.is_primary,
        })
    }
}

/// Monitor list captured at the last `monitor:change` check; `None` until a
/// frontend subscribes.
static WATCHED: Mutex<Option<Vec<Snapshot>>> = Mutex::new(None);

type ChangeHook = Box<dyn Fn() + Send + Sync>;

/// Installed by the event loop at startup: asks the main thread to re-read
/// the monitor list ([`refresh`]).
static CHANGE_HOOK: std::sync::OnceLock<ChangeHook> = std::sync::OnceLock::new();

pub(crate) fn set_change_hook(hook: ChangeHook) {
    if CHANGE_HOOK.set(hook).is_err() {
        crate::tynd_warn!("monitor change hook already installed");
    }
}

/// Called by the OS change sources (any thread).
fn notify_changed() {
    if let Some(hook) = CHANGE_HOOK.get() {
        hook();
    }
}

pub(super) fn dispatch(win: &Window, method: &str, args: &Value) -> DispatchResult {
    match method {
        "monitors" => Some(Ok(Value::Array(
            snapshots(win).iter().map(|(_, s)| s.to_json()).collect(),
        ))),
        "primaryMonitor" => Some(Ok(find(win, win.primary_monitor()))),
        "currentMonitor" => Some(Ok(find(win, win.current_monitor()))),
        "monitorFromPoint" => {
            let (x, y) = super::coordinates(args);
            Some(Ok(find(win, win.monitor_from_point(x, y))))
        },
        "globalCursorPosition" => Some(
            win.cursor_position()
                .map(|position| json!({ "x": position.x, "y": position.y }))
                .map_err(|error| format!("globalCursorPosition: {error}")),
        ),
        "subscribeMonitorChanges" => Some(subscribe(win).map(|()| Value::Null)),
        _ => None,
    }
}

/// Start watching the displays (idempotent). Main thread only.
fn subscribe(win: &Window) -> Result<(), String> {
    watch::install().map_err(|error| format!("subscribeMonitorChanges: {error}"))?;
    let mut watched = WATCHED.lock();
    if watched.is_none() {
        *watched = Some(snapshots(win).into_iter().map(|(_, s)| s).collect());
    }
    Ok(())
}

/// Re-read the monitor list and emit `monitor:change` when it differs from
/// the last one. No-op until a frontend subscribed. Main thread only.
pub(crate) fn refresh(win: &Window) {
    let mut watched = WATCHED.lock();
    let Some(previous) = watched.as_mut() else {
        return;
    };
    let current: Vec<Snapshot> = snapshots(win).into_iter().map(|(_, s)| s).collect();
    if let Some(payload) = diff(previous, &current) {
        *previous = current;
        drop(watched);
        crate::ipc::events::emit("monitor:change", &payload);
    }
}

/// `{ added, removed, changed, monitors }` when the lists differ, matched by
/// monitor id; `None` when nothing changed.
fn diff(previous: &[Snapshot], current: &[Snapshot]) -> Option<Value> {
    let by_id = |list: &[Snapshot], id: &str| list.iter().position(|s| s.id == id);
    let added: Vec<Value> = current
        .iter()
        .filter(|s| by_id(previous, &s.id).is_none())
        .map(Snapshot::to_json)
        .collect();
    let removed: Vec<Value> = previous
        .iter()
        .filter(|s| by_id(current, &s.id).is_none())
        .map(Snapshot::to_json)
        .collect();
    let changed: Vec<Value> = current
        .iter()
        .filter(|s| by_id(previous, &s.id).is_some_and(|index| previous[index] != **s))
        .map(Snapshot::to_json)
        .collect();
    if added.is_empty() && removed.is_empty() && changed.is_empty() {
        return None;
    }
    Some(json!({
        "added": added,
        "removed": removed,
        "changed": changed,
        "monitors": current.iter().map(Snapshot::to_json).collect::<Vec<_>>(),
    }))
}

/// Snapshot of `handle` within the full list (ids depend on the list on
/// Linux), or `null`.
fn find(win: &Window, handle: Option<MonitorHandle>) -> Value {
    let Some(handle) = handle else {
        return Value::Null;
    };
    snapshots(win)
        .into_iter()
        .find(|(candidate, _)| *candidate == handle)
        .map_or(Value::Null, |(_, s)| s.to_json())
}

fn snapshots(win: &Window) -> Vec<(MonitorHandle, Snapshot)> {
    let handles: Vec<MonitorHandle> = win.available_monitors().collect();
    let primary = win.primary_monitor();
    let names: Vec<Option<String>> = handles.iter().map(MonitorHandle::name).collect();
    handles
        .iter()
        .enumerate()
        .map(|(index, handle)| {
            let bounds = bounds(handle);
            let snapshot = Snapshot {
                id: monitor_id(handle, &names, index),
                name: names[index].clone(),
                bounds,
                work_area: work_area(handle).unwrap_or(bounds),
                scale: handle.scale_factor(),
                is_primary: primary.as_ref() == Some(handle),
            };
            (handle.clone(), snapshot)
        })
        .collect()
}

fn bounds(handle: &MonitorHandle) -> Rect {
    let position = handle.position();
    let size = handle.size();
    Rect {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    }
}

/// Windows: the adapter device name (`\\.\DISPLAY1`).
#[cfg(target_os = "windows")]
fn monitor_id(handle: &MonitorHandle, _names: &[Option<String>], _index: usize) -> String {
    use tao::platform::windows::MonitorHandleExtWindows;
    handle.native_id()
}

/// macOS: the `CGDirectDisplayID`.
#[cfg(target_os = "macos")]
fn monitor_id(handle: &MonitorHandle, _names: &[Option<String>], _index: usize) -> String {
    use tao::platform::macos::MonitorHandleExtMacOS;
    handle.native_id().to_string()
}

/// Linux: GDK 3 exposes no connector name, so the model name plus its rank
/// among monitors of the same model.
#[cfg(target_os = "linux")]
fn monitor_id(_handle: &MonitorHandle, names: &[Option<String>], index: usize) -> String {
    linux_monitor_id(names, index)
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn linux_monitor_id(names: &[Option<String>], index: usize) -> String {
    let name = names[index].as_deref().unwrap_or("monitor");
    let rank = names[..index]
        .iter()
        .filter(|other| other.as_deref().unwrap_or("monitor") == name)
        .count();
    format!("{name}#{rank}")
}

/// Work area of `handle` in physical pixels, `None` when the OS does not
/// report one (the full bounds are used then).
#[cfg(target_os = "windows")]
pub(crate) fn work_area(handle: &MonitorHandle) -> Option<Rect> {
    use tao::platform::windows::MonitorHandleExtWindows;
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};

    let monitor = handle.hmonitor() as HMONITOR;
    // SAFETY: `GetMonitorInfoW` writes into a zero-initialised MONITORINFO
    // whose `cbSize` is set as required; a stale handle makes it return 0.
    let work = unsafe {
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = u32::try_from(size_of::<MONITORINFO>()).ok()?;
        if GetMonitorInfoW(monitor, &raw mut info) == 0 {
            return None;
        }
        info.rcWork
    };
    Some(Rect {
        x: work.left,
        y: work.top,
        width: u32::try_from(work.right - work.left).ok()?,
        height: u32::try_from(work.bottom - work.top).ok()?,
    })
}

/// macOS: `NSScreen.visibleFrame` (minus menu bar and Dock), converted from
/// Cocoa points (bottom-left origin) to tao's physical top-left space via
/// its insets inside `NSScreen.frame`.
#[cfg(target_os = "macos")]
pub(crate) fn work_area(handle: &MonitorHandle) -> Option<Rect> {
    use objc2_app_kit::NSScreen;
    use tao::platform::macos::MonitorHandleExtMacOS;

    let screen = handle.ns_screen()?;
    // SAFETY: tao returns a live `NSScreen` pointer for this monitor; it is
    // only borrowed for the two property reads below.
    let screen: &NSScreen = unsafe { &*screen.cast::<NSScreen>() };
    let frame = screen.frame();
    let visible = screen.visibleFrame();
    let bounds = bounds(handle);
    Some(inset_work_area(
        bounds,
        handle.scale_factor(),
        [
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
        ],
        [
            visible.origin.x,
            visible.origin.y,
            visible.size.width,
            visible.size.height,
        ],
    ))
}

/// `bounds` shrunk by the insets of `visible` inside `frame` (both Cocoa
/// rects `[x, y, width, height]` in points, y growing upwards).
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "screen geometry is far inside i32 / u32 range"
)]
fn inset_work_area(bounds: Rect, scale: f64, frame: [f64; 4], visible: [f64; 4]) -> Rect {
    let left = (visible[0] - frame[0]) * scale;
    let top = ((frame[1] + frame[3]) - (visible[1] + visible[3])) * scale;
    Rect {
        x: bounds.x + left.round() as i32,
        y: bounds.y + top.round() as i32,
        width: (visible[2] * scale).round().max(0.0) as u32,
        height: (visible[3] * scale).round().max(0.0) as u32,
    }
}

/// Linux: `gdk_monitor_get_workarea` (application pixels) scaled like tao
/// scales the monitor geometry.
#[cfg(target_os = "linux")]
pub(crate) fn work_area(handle: &MonitorHandle) -> Option<Rect> {
    use gtk::gdk::prelude::MonitorExt;
    use tao::platform::unix::MonitorHandleExtUnix;

    let monitor = handle.gdk_monitor();
    let area = monitor.workarea();
    let scale = monitor.scale_factor();
    Some(Rect {
        x: area.x() * scale,
        y: area.y() * scale,
        width: u32::try_from(area.width() * scale).ok()?,
        height: u32::try_from(area.height() * scale).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn monitor(id: &str, width: u32, scale: f64) -> Snapshot {
        Snapshot {
            id: id.into(),
            name: Some(id.into()),
            bounds: rect(0, 0, width, 1080),
            work_area: rect(0, 0, width, 1040),
            scale,
            is_primary: id == "a",
        }
    }

    #[test]
    fn identical_lists_produce_no_event() {
        let list = vec![monitor("a", 1920, 1.0), monitor("b", 2560, 1.5)];
        assert!(diff(&list, &list.clone()).is_none());
    }

    #[test]
    fn added_removed_and_changed_monitors_are_reported() {
        let before = vec![monitor("a", 1920, 1.0), monitor("b", 2560, 1.5)];
        let mut resized = monitor("a", 1920, 1.25);
        resized.work_area = rect(0, 0, 1920, 1000);
        let after = vec![resized, monitor("c", 3840, 2.0)];
        let payload = diff(&before, &after).unwrap();
        assert_eq!(payload["added"][0]["id"], "c");
        assert_eq!(payload["removed"][0]["id"], "b");
        assert_eq!(payload["changed"][0]["id"], "a");
        assert_eq!(payload["changed"][0]["scale"], 1.25);
        assert_eq!(payload["changed"][0]["workArea"]["height"], 1000);
        assert_eq!(payload["monitors"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn snapshot_json_shape() {
        let value = monitor("a", 1920, 1.0).to_json();
        assert_eq!(value["position"], json!({ "x": 0, "y": 0 }));
        assert_eq!(value["size"], json!({ "width": 1920, "height": 1080 }));
        assert_eq!(
            value["workArea"],
            json!({ "x": 0, "y": 0, "width": 1920, "height": 1040 })
        );
        assert_eq!(value["isPrimary"], true);
    }

    #[test]
    fn linux_ids_rank_monitors_of_the_same_model() {
        let names = vec![Some("DELL".to_string()), None, Some("DELL".to_string())];
        assert_eq!(linux_monitor_id(&names, 0), "DELL#0");
        assert_eq!(linux_monitor_id(&names, 1), "monitor#0");
        assert_eq!(linux_monitor_id(&names, 2), "DELL#1");
    }

    #[test]
    fn macos_visible_frame_becomes_a_physical_work_area() {
        // Retina 1440x900 pt screen at the origin: 25 pt menu bar on top,
        // 70 pt Dock at the bottom.
        let bounds = rect(0, 0, 2880, 1800);
        let work = inset_work_area(
            bounds,
            2.0,
            [0.0, 0.0, 1440.0, 900.0],
            [0.0, 70.0, 1440.0, 805.0],
        );
        assert_eq!(work, rect(0, 50, 2880, 1610));

        // Secondary screen left of the primary, Dock on its left edge.
        let bounds = rect(-1920, 0, 1920, 1080);
        let work = inset_work_area(
            bounds,
            1.0,
            [-1920.0, 0.0, 1920.0, 1080.0],
            [-1856.0, 0.0, 1856.0, 1080.0],
        );
        assert_eq!(work, rect(-1856, 0, 1856, 1080));
    }
}
