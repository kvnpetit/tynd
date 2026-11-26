use std::collections::HashMap;

use serde_json::{json, Value};
use smallvec::SmallVec;
use tao::window::{Window, WindowId};
use wry::WebView;

use super::{child_webview::ChildWebviewEntry, yields::webview_for, SecondaryEntry, PRIMARY_LABEL};
use crate::{ipc::dispatch, webview::navigation};
use tynd_protocol::runtime::BackendCall;

use crate::webview::shim;

pub(super) fn smoke_duration_ms(raw: Option<&str>) -> u64 {
    raw.and_then(|value| value.parse::<u64>().ok())
        .map_or(1_500, |value| value.clamp(500, 60_000))
}

#[cfg(feature = "debug-protocol")]
pub(super) fn debug_window_info(label: &str, window: &Window) -> Value {
    let size = window.inner_size();
    let position = window
        .outer_position()
        .unwrap_or_else(|_| dpi::PhysicalPosition::new(0, 0));
    let info = json!({
        "label": label,
        "title": window.title(),
        "visible": window.is_visible(),
        "focused": window.is_focused(),
        "size": {
            "width": size.width,
            "height": size.height,
        },
        "position": {
            "x": position.x,
            "y": position.y,
        },
    });
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;

        let mut info = info;
        if let Some(object) = info.as_object_mut() {
            object.insert(
                "native_id".into(),
                Value::from(window.hwnd() as usize as u64),
            );
        }
        info
    }
    #[cfg(not(target_os = "windows"))]
    {
        info
    }
}

pub(super) fn remove_child_webviews_for_parent(
    parent_label: &str,
    child_webviews: &mut HashMap<String, ChildWebviewEntry>,
    call_tx: &crossbeam_channel::Sender<BackendCall>,
) {
    let labels: SmallVec<[String; 4]> = child_webviews
        .iter()
        .filter(|(_, entry)| entry.parent_label == parent_label)
        .map(|(label, _)| label.clone())
        .collect();
    for label in labels {
        if let Some(entry) = child_webviews.remove(&label) {
            let parent = entry.parent_label.clone();
            drop(entry);
            dispatch::cancel_calls_for_label(&label, call_tx);
            navigation::clear_remote_origin(&label);
            crate::ipc::events::emit(
                "webview:closed",
                &json!({ "label": label, "parent": parent }),
            );
        }
    }
}

pub(super) fn window_for_label<'a>(
    label: &str,
    primary: &'a Window,
    secondaries: &'a HashMap<String, SecondaryEntry>,
) -> Option<&'a Window> {
    if label == PRIMARY_LABEL {
        Some(primary)
    } else {
        secondaries.get(label).map(|entry| &entry.window)
    }
}

pub(super) fn requested_label<'a>(args: &'a Value, operation: &str) -> Result<&'a str, String> {
    args.get("label")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
        .ok_or_else(|| format!("{operation}: missing non-empty 'label'"))
}

pub(super) fn parse_zoom_level(args: &Value, operation: &str) -> Result<f64, String> {
    args.get("level")
        .and_then(Value::as_f64)
        .filter(|level| level.is_finite() && *level > 0.0)
        .ok_or_else(|| format!("{operation}: 'level' must be a positive finite number"))
}

pub(super) fn result_reply(result: Result<Value, String>) -> (bool, Value) {
    match result {
        Ok(value) => (true, value),
        Err(error) => (false, Value::String(error)),
    }
}

pub(super) fn child_webview_operation(
    args: &Value,
    operation: &str,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
    task: impl FnOnce(&WebView) -> wry::Result<()>,
) -> Result<Value, String> {
    let label = requested_label(args, operation)?;
    let webview = &child_webviews
        .get(label)
        .ok_or_else(|| format!("webview '{label}' not found"))?
        .webview;
    task(webview).map_err(|error| format!("{operation}: {error}"))?;
    Ok(Value::Null)
}

pub(super) fn broadcast_script(
    script: &str,
    primary: &WebView,
    secondaries: &HashMap<String, SecondaryEntry>,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
) {
    let _ = primary.evaluate_script(script);
    for entry in secondaries.values() {
        let _ = entry.webview.evaluate_script(script);
    }
    for entry in child_webviews.values() {
        let _ = entry.webview.evaluate_script(script);
    }
}
/// Deliver `script` only to the webviews `scope` selects. A webview counts as
/// app-local while [`crate::webview::handlers::is_app_local_webview`] holds.
pub(super) fn broadcast_script_scoped(
    script: &str,
    scope: &crate::ipc::events::EventScope,
    primary: &WebView,
    secondaries: &HashMap<String, SecondaryEntry>,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
) {
    let mut labels: Vec<&str> = Vec::with_capacity(1 + secondaries.len() + child_webviews.len());
    labels.push(PRIMARY_LABEL);
    labels.extend(secondaries.keys().map(String::as_str));
    labels.extend(child_webviews.keys().map(String::as_str));
    let targets = super::os_event_queue::scope_targets(
        scope,
        &labels,
        crate::webview::handlers::is_app_local_webview,
    );
    for label in targets {
        if let Some(webview) = webview_for(label, primary, secondaries, child_webviews) {
            let _ = webview.evaluate_script(script);
        }
    }
}

/// Deliver `script` to the webviews that currently show remote content: the
/// complement of [`broadcast_script_scoped`] with `EventScope::Local`.
pub(super) fn broadcast_script_remote(
    script: &str,
    primary: &WebView,
    secondaries: &HashMap<String, SecondaryEntry>,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
) {
    let labels = secondaries
        .keys()
        .chain(child_webviews.keys())
        .map(String::as_str)
        .chain(std::iter::once(PRIMARY_LABEL));
    for label in labels {
        if crate::webview::handlers::is_app_local_webview(label) {
            continue;
        }
        if let Some(webview) = webview_for(label, primary, secondaries, child_webviews) {
            let _ = webview.evaluate_script(script);
        }
    }
}

/// Route an `os_call` reply (`OsResult`, `TrayCmd`, `MenuCmd`) back to the
/// originating webview. Same shape across all three handler arms — extracted
/// so the event-loop body stays a flat dispatch table instead of repeating
/// the eval-script + warn-on-failure boilerplate per arm.
pub(super) fn dispatch_os_reply(
    label: &str,
    id: &str,
    ok: bool,
    value: &Value,
    primary: &WebView,
    secondaries: &HashMap<String, SecondaryEntry>,
    child_webviews: &HashMap<String, ChildWebviewEntry>,
) {
    let script = shim::eval_os_result(id, ok, value);
    let Some(webview) = webview_for(label, primary, secondaries, child_webviews) else {
        crate::tynd_warn!("dropping ipc reply for closed webview id={id} label={label:?}");
        return;
    };
    if let Err(e) = webview.evaluate_script(&script) {
        crate::tynd_warn!("ipc reply eval failed for id={id} label={label:?}: {e}");
    }
}

/// Label of the window `id`, or `None` for a window the registry no longer
/// knows (an event queued for a window that was already dropped). Callers
/// ignore such events instead of attributing them to the primary window.
pub(super) fn label_for(id: WindowId, map: &HashMap<WindowId, String>) -> Option<String> {
    map.get(&id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_window_ids_have_no_label() {
        // SAFETY: the dummy id is only used as a map key, never as a real window.
        let id = unsafe { WindowId::dummy() };
        let mut map = HashMap::new();
        assert_eq!(label_for(id, &map), None);
        map.insert(id, PRIMARY_LABEL.to_string());
        assert_eq!(label_for(id, &map).as_deref(), Some(PRIMARY_LABEL));
    }
}
