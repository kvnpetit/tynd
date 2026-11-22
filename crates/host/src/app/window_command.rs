use super::{
    child_webview_commands::{dispatch_child_webview_command, ChildWebviewCommandContext},
    dispatch_support::{dispatch_os_reply, result_reply},
    runtime::WindowRegistry,
    secondary_window::create_secondary,
    source_webview_commands::{dispatch_source_webview_command, SourceWebviewCommandContext},
    window_events::window_created,
    window_messages::{deliver_window_message, parse_window_message},
    UserEvent, PRIMARY_LABEL,
};
use tynd_protocol::runtime::BackendCall;

use crate::{api, api::menu};
use crossbeam_channel::Sender;
use serde_json::Value;
use tao::{
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::Window,
};

pub(super) struct WindowCommandContext<'a> {
    /// Origin of the calling frame (`None` = app-local content).
    pub(super) caller_origin: Option<&'a str>,
    pub(super) target: &'a EventLoopWindowTarget<UserEvent>,
    pub(super) debug: bool,
    pub(super) call_tx: &'a Sender<BackendCall>,
    pub(super) proxy: &'a EventLoopProxy<UserEvent>,
    pub(super) dev_url: Option<&'a str>,
    pub(super) frontend_dir: Option<&'a str>,
    pub(super) windows: &'a mut WindowRegistry,
}

type CommandResult = Result<Value, String>;

pub(super) fn handle_window_command(
    mut context: WindowCommandContext<'_>,
    label: String,
    id: String,
    method: String,
    args: Value,
) {
    if context.windows.webview_for(&label).is_none() {
        crate::tynd_warn!("dropping window command '{method}' from closed webview '{label}'");
        return;
    }

    // Session commands (evaluate, PDF, capture...) answer asynchronously.
    if crate::webview::session::is_session_method(&method) {
        let reply = crate::webview::session::Reply::new(context.proxy, &label, &id, &method);
        let windows = &*context.windows;
        crate::webview::session::dispatch(
            &method,
            &label,
            context.caller_origin,
            &args,
            |target| windows.webview_for(target),
            reply,
        );
        return;
    }

    // Child webviews have their own IPC/routing label, but native
    // window operations still target the containing tao window.
    let source_window_label = context
        .windows
        .child_webviews
        .get(&label)
        .map_or_else(|| label.clone(), |entry| entry.parent_label.clone());

    let result = dispatch_window_command(
        &mut context,
        &label,
        &source_window_label,
        method.as_str(),
        &args,
    );
    let (ok, value) = result_reply(result);
    dispatch_os_reply(
        &label,
        &id,
        ok,
        &value,
        &context.windows.webview,
        &context.windows.secondaries,
        &context.windows.child_webviews,
    );
}

/// A window command from the trusted backend. `args.window` names the target
/// window (default: the primary one); the backend has its own event emitter,
/// so window messaging stays a renderer API.
pub(super) fn run_backend_window_command(
    mut context: WindowCommandContext<'_>,
    method: &str,
    args: &Value,
) -> CommandResult {
    if matches!(method, "emit" | "emitTo") {
        return Err(format!(
            "window.{method} is a frontend API; in the backend use createEmitter().emit / emitTo"
        ));
    }
    let label = match args.get("window") {
        None | Some(Value::Null) => PRIMARY_LABEL,
        Some(Value::String(label)) => label.as_str(),
        Some(_) => return Err(format!("window.{method}: 'window' must be a window label")),
    }
    .to_owned();
    if context.windows.window_for(&label).is_none() {
        return Err(format!("window '{label}' not found"));
    }
    dispatch_window_command(&mut context, &label, &label, method, args)
}

fn dispatch_window_command(
    context: &mut WindowCommandContext<'_>,
    label: &str,
    source_window_label: &str,
    method: &str,
    args: &Value,
) -> CommandResult {
    match method {
        "create" => create_window(context, args),
        "close" => close_window(context, args),
        "all" => Ok(window_labels(context)),
        "emit" | "emitTo" => {
            api::require_multiwindow()?;
            let message = parse_window_message(method, args, context.caller_origin)?;
            deliver_window_message(context.windows, label, &message)
        },
        _ => {
            let child_result = dispatch_child_webview_command(
                method,
                source_window_label,
                args,
                ChildWebviewCommandContext {
                    debug: context.debug,
                    caller_webview: label,
                    caller_origin: context.caller_origin,
                    call_tx: context.call_tx,
                    proxy: context.proxy,
                    dev_url: context.dev_url,
                    frontend_dir: context.frontend_dir,
                    windows: context.windows,
                },
            );
            if let Some(result) = child_result {
                return result;
            }

            // `args.window` retargets a window method at another window
            // (`multiwindow.getWindow(label)` handles).
            let target = command_target(context, source_window_label, method, args)?;
            let (label, source_window_label) = match target.as_deref() {
                Some(target) => (target, target),
                None => (label, source_window_label),
            };
            if method == "setMenu" {
                return set_window_menu(context, source_window_label, args);
            }

            let source_result = dispatch_source_webview_command(
                method,
                label,
                args,
                SourceWebviewCommandContext {
                    windows: context.windows,
                },
            );
            if let Some(result) = source_result {
                return result;
            }

            if let Some(state) = child_frame_state(method, label, source_window_label) {
                return Ok(state);
            }
            if method == "showContextMenu" {
                show_context_menu(context, source_window_label, args)
            } else {
                dispatch_native_window_command(context, source_window_label, method, args)
            }
        },
    }
}
/// The window named by `args.window` when it is not the caller's own. Driving
/// another window needs the multiwindow API and app-local content: remote
/// pages only ever drive the window they run in.
fn command_target(
    context: &WindowCommandContext<'_>,
    source_window_label: &str,
    method: &str,
    args: &Value,
) -> Result<Option<String>, String> {
    let target = match args.get("window") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(target)) => target,
        Some(_) => return Err(format!("window.{method}: 'window' must be a window label")),
    };
    if target == source_window_label {
        return Ok(None);
    }
    api::require_multiwindow()?;
    if context.caller_origin.is_some() {
        return Err(format!(
            "window.{method}: remote content can only drive its own window"
        ));
    }
    if target != PRIMARY_LABEL && !context.windows.secondaries.contains_key(target) {
        return Err(format!("window '{target}' not found"));
    }
    Ok(Some(target.clone()))
}

fn set_window_menu(
    context: &mut WindowCommandContext<'_>,
    window_label: &str,
    args: &Value,
) -> CommandResult {
    if window_label == PRIMARY_LABEL {
        menu::set_primary_window_menu(&context.windows.native_window, args)?;
    } else {
        let entry = context
            .windows
            .secondaries
            .get_mut(window_label)
            .ok_or_else(|| format!("window '{window_label}' not found"))?;
        menu::set_secondary_window_menu(&entry.window, &mut entry.menu, args)?;
    }
    Ok(Value::Null)
}

fn create_window(context: &mut WindowCommandContext<'_>, args: &Value) -> CommandResult {
    api::require_multiwindow()?;
    let (new_label, entry) = create_secondary(
        context.target,
        args,
        context.debug,
        context.call_tx,
        context.proxy,
        context.dev_url,
        context.frontend_dir,
        &context.windows.webview_app_name,
        &context.windows.native_window,
        &context.windows.secondaries,
        &context.windows.child_webviews,
    )?;
    context
        .windows
        .window_id_to_label
        .insert(entry.window.id(), new_label.clone());
    if args
        .get("hideOnFocusLost")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        context
            .windows
            .hide_on_focus_lost
            .insert(new_label.clone(), true);
    }
    if args
        .get("hideOnEscape")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        context
            .windows
            .hide_on_escape
            .insert(new_label.clone(), true);
    }
    context.windows.secondaries.insert(new_label.clone(), entry);
    window_created(context.call_tx, &new_label);
    Ok(Value::Null)
}

fn close_window(context: &WindowCommandContext<'_>, args: &Value) -> CommandResult {
    api::require_multiwindow()?;
    let target_label = args
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if target_label == PRIMARY_LABEL {
        return Err("cannot close the primary window".into());
    }
    if !context.windows.secondaries.contains_key(&target_label) {
        return Err(format!("window '{target_label}' not found"));
    }
    context
        .proxy
        .send_event(UserEvent::CloseSecondaryWindow {
            label: target_label,
        })
        .map_err(|error| format!("close: queue close: {error}"))?;
    Ok(Value::Null)
}

fn window_labels(context: &WindowCommandContext<'_>) -> Value {
    let mut labels = Vec::with_capacity(context.windows.secondaries.len() + 1);
    labels.push(Value::String(PRIMARY_LABEL.into()));
    labels.extend(
        context
            .windows
            .secondaries
            .keys()
            .cloned()
            .map(Value::String),
    );
    Value::Array(labels)
}

fn command_window<'a>(
    context: &'a WindowCommandContext<'_>,
    source_window_label: &str,
) -> Result<&'a Window, String> {
    context
        .windows
        .window_for(source_window_label)
        .ok_or_else(|| format!("window '{source_window_label}' not found"))
}

/// `frameState` from a child webview: the injected frame script measures the
/// child's own edges, so it must never start resizing the parent window.
fn child_frame_state(method: &str, label: &str, source_window_label: &str) -> Option<Value> {
    (method == "frameState" && label != source_window_label)
        .then(|| serde_json::json!({ "edgeResizable": false }))
}

fn show_context_menu(
    context: &WindowCommandContext<'_>,
    source_window_label: &str,
    args: &Value,
) -> CommandResult {
    let window = command_window(context, source_window_label)?;
    menu::show_context_menu(window, args)?;
    Ok(Value::Null)
}

fn dispatch_native_window_command(
    context: &WindowCommandContext<'_>,
    source_window_label: &str,
    method: &str,
    args: &Value,
) -> CommandResult {
    let window = command_window(context, source_window_label)?;
    if method == "cancelClose" {
        super::close_guard::cancel(source_window_label);
        return Ok(Value::Null);
    }
    api::window_cmd::dispatch(window, method, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_webviews_never_edge_resize_their_parent() {
        assert_eq!(
            child_frame_state("frameState", "panel", "main"),
            Some(serde_json::json!({ "edgeResizable": false }))
        );
        assert_eq!(child_frame_state("frameState", "main", "main"), None);
        assert_eq!(child_frame_state("minimize", "panel", "main"), None);
    }
}
