//! Native file / message / confirm dialogs. File pickers wrap `rfd`; message
//! boxes use the native API of each OS (task dialog, `NSAlert`,
//! `GtkMessageDialog`) because `rfd` has no checkbox, no default / cancel
//! button and at most three custom labels.

use rfd::{AsyncFileDialog, FileHandle};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;
use crate::util::block_on;

mod message;
#[cfg(all(unix, not(target_os = "macos")))]
mod message_linux;
#[cfg(target_os = "macos")]
mod message_macos;
#[cfg(target_os = "windows")]
mod message_windows;
#[cfg(target_os = "windows")]
mod owner_windows;

use message::{Buttons, MessageArgs, MessageOutcome, MessageSpec};

/// File-extension filter passed to the open / save dialogs.
#[derive(Debug, Deserialize)]
pub(crate) struct FileFilter {
    /// Display name shown to the user (e.g. `"Images"`).
    pub(crate) name: String,
    /// File extensions without the leading dot (e.g. `["png", "jpg"]`).
    pub(crate) extensions: Vec<String>,
}

/// Options every file picker shares.
#[derive(Debug, Default)]
struct PickerOptions {
    title: Option<String>,
    default_dir: Option<String>,
    can_create_directories: Option<bool>,
    show_hidden_files: Option<bool>,
    parent: Option<bool>,
}

/// Stateless dispatch marker.
#[derive(Debug, Default)]
pub(crate) struct DialogApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("dialog", |method, args| dispatch(method, &args));

#[tynd::handler(api = "dialog")]
impl DialogApi {
    /// Open a file picker and return the selected path, or `null` on
    /// user cancel.
    pub fn open_file(
        title: Option<String>,
        default_dir: Option<String>,
        filters: Option<Vec<FileFilter>>,
        can_create_directories: Option<bool>,
        show_hidden_files: Option<bool>,
        parent: Option<bool>,
        grant_access: Option<bool>,
    ) -> Result<Value, String> {
        let grant = grant_requested("openFile", grant_access)?;
        let options = PickerOptions {
            title,
            default_dir,
            can_create_directories,
            show_hidden_files,
            parent,
        };
        let d = apply_filters(base_file_dialog(options), filters);
        let picked = block_on(d.pick_file());
        grant_picked("openFile", grant, picked.iter(), false)?;
        Ok(path_or_null(picked.as_ref()))
    }

    /// Open a multi-select file picker. Returns the selected paths
    /// (possibly empty) or `null` on user cancel.
    pub fn open_files(
        title: Option<String>,
        default_dir: Option<String>,
        filters: Option<Vec<FileFilter>>,
        can_create_directories: Option<bool>,
        show_hidden_files: Option<bool>,
        parent: Option<bool>,
        grant_access: Option<bool>,
    ) -> Result<Value, String> {
        let grant = grant_requested("openFiles", grant_access)?;
        let options = PickerOptions {
            title,
            default_dir,
            can_create_directories,
            show_hidden_files,
            parent,
        };
        let d = apply_filters(base_file_dialog(options), filters);
        let picked = block_on(d.pick_files());
        grant_picked("openFiles", grant, picked.iter().flatten(), false)?;
        Ok(paths_or_null(picked))
    }

    /// Open a directory picker. Returns the selected path, or `null`
    /// on user cancel.
    pub fn open_directory(
        title: Option<String>,
        default_dir: Option<String>,
        can_create_directories: Option<bool>,
        show_hidden_files: Option<bool>,
        parent: Option<bool>,
        grant_access: Option<bool>,
    ) -> Result<Value, String> {
        let grant = grant_requested("openDirectory", grant_access)?;
        let d = base_file_dialog(PickerOptions {
            title,
            default_dir,
            can_create_directories,
            show_hidden_files,
            parent,
        });
        let picked = block_on(d.pick_folder());
        grant_picked("openDirectory", grant, picked.iter(), false)?;
        Ok(path_or_null(picked.as_ref()))
    }

    /// Open a multi-select directory picker. Returns the selected paths
    /// or `null` on user cancel.
    pub fn open_directories(
        title: Option<String>,
        default_dir: Option<String>,
        can_create_directories: Option<bool>,
        show_hidden_files: Option<bool>,
        parent: Option<bool>,
        grant_access: Option<bool>,
    ) -> Result<Value, String> {
        let grant = grant_requested("openDirectories", grant_access)?;
        let d = base_file_dialog(PickerOptions {
            title,
            default_dir,
            can_create_directories,
            show_hidden_files,
            parent,
        });
        let picked = block_on(d.pick_folders());
        grant_picked("openDirectories", grant, picked.iter().flatten(), false)?;
        Ok(paths_or_null(picked))
    }

    /// Open a save-file dialog and return the chosen path, or `null`
    /// on user cancel. Every OS asks before overwriting an existing file.
    #[allow(
        clippy::too_many_arguments,
        reason = "one parameter per wire option of dialog.saveFile"
    )]
    pub fn save_file(
        title: Option<String>,
        default_name: Option<String>,
        default_dir: Option<String>,
        filters: Option<Vec<FileFilter>>,
        can_create_directories: Option<bool>,
        show_hidden_files: Option<bool>,
        parent: Option<bool>,
        grant_access: Option<bool>,
    ) -> Result<Value, String> {
        let grant = grant_requested("saveFile", grant_access)?;
        let mut d = base_file_dialog(PickerOptions {
            title,
            default_dir,
            can_create_directories,
            show_hidden_files,
            parent,
        });
        if let Some(n) = default_name {
            d = d.set_file_name(&n);
        }
        d = apply_filters(d, filters);
        let picked = block_on(d.save_file());
        grant_picked("saveFile", grant, picked.iter(), true)?;
        Ok(path_or_null(picked.as_ref()))
    }

    /// Show a message box and return `{ response, checkboxChecked }`:
    /// the index of the clicked button (`cancelId` when dismissed) and the
    /// final checkbox state. `kind` is `none`, `info` (default), `warning`,
    /// `error` or `question`; without `buttons` a single localized OK is
    /// shown.
    #[allow(
        clippy::too_many_arguments,
        reason = "one parameter per wire option of dialog.message"
    )]
    pub fn message(
        message: String,
        title: Option<String>,
        detail: Option<String>,
        kind: Option<String>,
        buttons: Option<Vec<String>>,
        default_id: Option<u32>,
        cancel_id: Option<u32>,
        checkbox_label: Option<String>,
        checkbox_checked: Option<bool>,
        parent: Option<bool>,
    ) -> Result<Value, String> {
        let args = MessageArgs {
            message,
            title,
            detail,
            kind,
            buttons,
            default_id,
            cancel_id,
            checkbox_label,
            checkbox_checked,
            parent,
        };
        let outcome = run_message("message", args, Buttons::Ok, "Message")?;
        Ok(json!({
            "response": outcome.response,
            "checkboxChecked": outcome.checkbox_checked,
        }))
    }

    /// Show an OK/Cancel confirmation dialog. Returns `true` when the
    /// user pressed OK, `false` for Cancel or any close.
    pub fn confirm(
        message: String,
        title: Option<String>,
        detail: Option<String>,
        kind: Option<String>,
        parent: Option<bool>,
    ) -> Result<bool, String> {
        let args = MessageArgs {
            message,
            title,
            detail,
            kind,
            parent,
            ..MessageArgs::default()
        };
        let outcome = run_message("confirm", args, Buttons::OkCancel, "Confirm")?;
        Ok(outcome.response == 0)
    }
}

/// `grantAccess: true`, checked before any dialog shows: remote content may
/// never widen the filesystem scope, even through the user.
fn grant_requested(method: &str, grant_access: Option<bool>) -> Result<bool, String> {
    if grant_access != Some(true) {
        return Ok(false);
    }
    if crate::ipc::owner::current().is_some_and(|caller| caller.origin().is_some()) {
        return Err(IpcError::new(
            "dialog",
            method,
            "grantAccess is only available to app content, not to remote pages",
        )
        .into_string());
    }
    Ok(true)
}

/// Grant the paths the user picked (`grantAccess: true`): read-only for the
/// open pickers (a picked directory with its whole subtree), read and write
/// for the file chosen in a save dialog.
fn grant_picked<'a>(
    method: &str,
    grant: bool,
    picked: impl Iterator<Item = &'a FileHandle>,
    write: bool,
) -> Result<(), String> {
    if !grant {
        return Ok(());
    }
    for handle in picked {
        let path = handle.path();
        let text = path.to_str().ok_or_else(|| {
            IpcError::new("dialog", method, "picked path is not valid UTF-8").into_string()
        })?;
        crate::security::grants::grant_path(text, path.is_dir(), write)
            .map_err(|e| IpcError::new("dialog", method, e).into_string())?;
    }
    Ok(())
}

/// Validate `args` and show the native message box.
fn run_message(
    method: &str,
    args: MessageArgs,
    preset: Buttons,
    default_title: &str,
) -> Result<MessageOutcome, String> {
    let dialog_err = |e: String| IpcError::new("dialog", method, e).into_string();
    let spec = MessageSpec::parse(args, preset, default_title).map_err(dialog_err)?;
    show_message(&spec).map_err(dialog_err)
}

fn show_message(spec: &MessageSpec) -> Result<MessageOutcome, String> {
    #[cfg(target_os = "windows")]
    {
        message_windows::show(spec)
    }
    #[cfg(target_os = "macos")]
    {
        message_macos::show(spec)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        message_linux::show(spec)
    }
}

fn path_value(handle: &FileHandle) -> Value {
    Value::String(handle.path().to_string_lossy().into_owned())
}

/// Selected path, or `null` when the user cancelled.
fn path_or_null(handle: Option<&FileHandle>) -> Value {
    handle.map_or(Value::Null, path_value)
}

/// Selected paths, or `null` when the user cancelled.
fn paths_or_null(handles: Option<Vec<FileHandle>>) -> Value {
    handles.map_or(Value::Null, |handles| {
        Value::Array(handles.iter().map(path_value).collect())
    })
}

/// File dialog with the options every picker shares.
fn base_file_dialog(options: PickerOptions) -> AsyncFileDialog {
    let mut d = AsyncFileDialog::new();
    if let Some(t) = options.title {
        d = d.set_title(&t);
    }
    if let Some(p) = options.default_dir {
        d = d.set_directory(&p);
    }
    if let Some(c) = options.can_create_directories {
        d = d.set_can_create_directories(c);
    }
    if let Some(show) = options.show_hidden_files {
        d = d.set_show_hidden_files(show);
    }
    // Modal to the app's active window. Windows only: on macOS the panel is
    // app-modal (a sheet needs the NSWindow, owned by the main thread) and
    // the Linux portal/zenity pickers take no owner from a pool thread.
    #[cfg(target_os = "windows")]
    if options.parent.unwrap_or(true) {
        if let Some(owner) = owner_windows::OwnerHandle::foreground() {
            d = d.set_parent(&owner);
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = options.parent;
    d
}

fn apply_filters(mut d: AsyncFileDialog, filters: Option<Vec<FileFilter>>) -> AsyncFileDialog {
    if let Some(filters) = filters {
        for filter in &filters {
            let exts: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
            d = d.add_filter(&filter.name, &exts);
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{dispatch, grant_requested};
    use crate::ipc::owner;

    #[test]
    fn grant_access_is_refused_to_remote_content_before_any_dialog() {
        for method in [
            "openFile",
            "openFiles",
            "openDirectory",
            "openDirectories",
            "saveFile",
        ] {
            let error = owner::with_caller_trust("panel", false, || {
                dispatch(method, &json!({ "grantAccess": true }))
            })
            .expect_err("remote grantAccess");
            assert!(error.contains("grantAccess"), "{method}: {error}");
        }
        assert_eq!(
            owner::with_caller_trust("main", true, || grant_requested("openFile", Some(true))),
            Ok(true)
        );
        assert_eq!(grant_requested("openFile", Some(true)), Ok(true));
        assert_eq!(
            owner::with_caller_trust("panel", false, || grant_requested("openFile", None)),
            Ok(false)
        );
    }

    #[test]
    fn message_rejects_invalid_arguments_before_showing_anything() {
        let err = dispatch(
            "message",
            &json!({ "message": "x", "buttons": ["A"], "defaultId": 4 }),
        )
        .expect_err("out of range");
        assert!(err.starts_with("dialog.message:"), "{err}");
        assert!(err.contains("defaultId 4"), "{err}");

        let err =
            dispatch("confirm", &json!({ "message": "x", "kind": "loud" })).expect_err("bad kind");
        assert!(err.starts_with("dialog.confirm:"), "{err}");

        assert!(dispatch("message", &json!({ "title": "no message" })).is_err());
    }
}
