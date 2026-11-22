//! Native file / message / confirm dialogs — wraps `rfd` for a uniform
//! TS API across platforms.

use rfd::{
    AsyncFileDialog, AsyncMessageDialog, FileHandle, MessageButtons, MessageDialogResult,
    MessageLevel,
};
use serde::Deserialize;
use serde_json::Value;

use crate::api::HostApi;
use crate::tynd;
use crate::util::block_on;

/// File-extension filter passed to the open / save dialogs.
#[derive(Debug, Deserialize)]
pub(crate) struct FileFilter {
    /// Display name shown to the user (e.g. `"Images"`).
    pub(crate) name: String,
    /// File extensions without the leading dot (e.g. `["png", "jpg"]`).
    pub(crate) extensions: Vec<String>,
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
    ) -> Value {
        let d = build_open_dialog(title, default_dir, filters, can_create_directories);
        path_or_null(block_on(d.pick_file()).as_ref())
    }

    /// Open a multi-select file picker. Returns the selected paths
    /// (possibly empty) or `null` on user cancel.
    pub fn open_files(
        title: Option<String>,
        default_dir: Option<String>,
        filters: Option<Vec<FileFilter>>,
        can_create_directories: Option<bool>,
    ) -> Value {
        let d = build_open_dialog(title, default_dir, filters, can_create_directories);
        match block_on(d.pick_files()) {
            None => Value::Null,
            Some(handles) => Value::Array(handles.iter().map(path_value).collect()),
        }
    }

    /// Open a directory picker. Returns the selected path, or `null`
    /// on user cancel.
    pub fn open_directory(
        title: Option<String>,
        default_dir: Option<String>,
        can_create_directories: Option<bool>,
    ) -> Value {
        let d = base_file_dialog(title, default_dir, can_create_directories);
        path_or_null(block_on(d.pick_folder()).as_ref())
    }

    /// Open a save-file dialog and return the chosen path, or `null`
    /// on user cancel.
    pub fn save_file(
        title: Option<String>,
        default_name: Option<String>,
        default_dir: Option<String>,
        filters: Option<Vec<FileFilter>>,
        can_create_directories: Option<bool>,
    ) -> Value {
        let mut d = base_file_dialog(title, default_dir, can_create_directories);
        if let Some(n) = default_name {
            d = d.set_file_name(&n);
        }
        d = apply_filters(d, filters);
        path_or_null(block_on(d.save_file()).as_ref())
    }

    /// Show a single-button informational message dialog. `kind`
    /// accepts `"info"` (default), `"warning"`, or `"error"`.
    pub fn message(message: String, title: Option<String>, kind: Option<String>) {
        let level = match kind.as_deref().unwrap_or("info") {
            "warning" => MessageLevel::Warning,
            "error" => MessageLevel::Error,
            _ => MessageLevel::Info,
        };
        block_on(
            AsyncMessageDialog::new()
                .set_title(title.unwrap_or_else(|| "Message".into()))
                .set_description(&message)
                .set_level(level)
                .set_buttons(MessageButtons::Ok)
                .show(),
        );
    }

    /// Show an OK/Cancel confirmation dialog. Returns `true` when the
    /// user pressed OK, `false` for Cancel or any close.
    pub fn confirm(message: String, title: Option<String>) -> bool {
        let result = block_on(
            AsyncMessageDialog::new()
                .set_title(title.unwrap_or_else(|| "Confirm".into()))
                .set_description(&message)
                .set_level(MessageLevel::Info)
                .set_buttons(MessageButtons::OkCancel)
                .show(),
        );
        result == MessageDialogResult::Ok
    }
}

fn path_value(handle: &FileHandle) -> Value {
    Value::String(handle.path().to_string_lossy().into_owned())
}

/// Selected path, or `null` when the user cancelled.
fn path_or_null(handle: Option<&FileHandle>) -> Value {
    handle.map_or(Value::Null, path_value)
}

/// File dialog with the options every picker shares.
fn base_file_dialog(
    title: Option<String>,
    default_dir: Option<String>,
    can_create_directories: Option<bool>,
) -> AsyncFileDialog {
    let mut d = AsyncFileDialog::new();
    if let Some(t) = title {
        d = d.set_title(&t);
    }
    if let Some(p) = default_dir {
        d = d.set_directory(&p);
    }
    if let Some(c) = can_create_directories {
        d = d.set_can_create_directories(c);
    }
    d
}

fn build_open_dialog(
    title: Option<String>,
    default_dir: Option<String>,
    filters: Option<Vec<FileFilter>>,
    can_create_directories: Option<bool>,
) -> AsyncFileDialog {
    apply_filters(
        base_file_dialog(title, default_dir, can_create_directories),
        filters,
    )
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
