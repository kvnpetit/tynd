//! Windows message box: `TaskDialogIndirect` from Common Controls v6.
//!
//! The task dialog is the only stock Win32 box with custom button labels, a
//! default / cancel button and a checkbox ("verification" text). It lives in
//! comctl32 v6, which a process only gets through an activation context: the
//! host binary has no Common-Controls manifest (linking the import directly
//! would fail at load time against comctl32 v5), so the call borrows the
//! manifest Windows ships as resource 124 of `shell32.dll`, loads comctl32
//! under it and resolves `TaskDialogIndirect` at runtime.

use std::{ffi::c_void, ptr, sync::OnceLock};

use windows_sys::{
    core::{BOOL, HRESULT, PCWSTR},
    Win32::{
        Foundation::{GetLastError, HANDLE, HWND, INVALID_HANDLE_VALUE},
        System::{
            ApplicationInstallationAndServicing::{
                ActivateActCtx, CreateActCtxW, DeactivateActCtx, ACTCTXW,
            },
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            SystemInformation::GetSystemDirectoryW,
        },
        UI::{
            Controls::{
                TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON,
                TDCBF_OK_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
                TDF_USE_HICON_MAIN, TDF_VERIFICATION_FLAG_CHECKED, TD_ERROR_ICON,
                TD_INFORMATION_ICON, TD_WARNING_ICON,
            },
            WindowsAndMessaging::{LoadIconW, IDCANCEL, IDI_QUESTION, IDOK},
        },
    },
};

use super::{
    message::{Buttons, Kind, MessageOutcome, MessageSpec},
    owner_windows,
};

/// `ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID` (WinBase.h).
const ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID: u32 = 0x4;
/// `ACTCTX_FLAG_RESOURCE_NAME_VALID` (WinBase.h).
const ACTCTX_FLAG_RESOURCE_NAME_VALID: u32 = 0x8;
/// Resource id of the Common Controls v6 manifest embedded in `shell32.dll`.
const SHELL32_COMCTL6_MANIFEST: usize = 124;
/// Id of custom button `i` is `CUSTOM_BUTTON_BASE + i` (clear of `IDOK`...).
const CUSTOM_BUTTON_BASE: i32 = 100;

type TaskDialogIndirectFn = unsafe extern "system" fn(
    config: *const TASKDIALOGCONFIG,
    button: *mut i32,
    radio_button: *mut i32,
    verification_checked: *mut BOOL,
) -> HRESULT;

/// Activation context and entry point, resolved once per process.
struct Comctl6 {
    /// Activation context handle, kept as an integer so the cache is `Sync`.
    /// Never released: it must outlive every dialog.
    context: isize,
    task_dialog: TaskDialogIndirectFn,
}

static COMCTL6: OnceLock<Result<Comctl6, String>> = OnceLock::new();

/// NUL-terminated UTF-16 copy of `text` (inner NULs are rejected upstream).
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Activated context, deactivated on drop (also on unwind).
struct Activation(usize);

impl Activation {
    fn enter(context: HANDLE) -> Result<Self, String> {
        let mut cookie = 0usize;
        // SAFETY: `context` is a live activation context; `cookie` is a
        // valid out pointer.
        if unsafe { ActivateActCtx(context, &raw mut cookie) } == 0 {
            // SAFETY: no arguments.
            let code = unsafe { GetLastError() };
            return Err(format!("ActivateActCtx failed (error {code})"));
        }
        Ok(Self(cookie))
    }
}

impl Drop for Activation {
    fn drop(&mut self) {
        // SAFETY: `self.0` is the cookie of the activation made on this
        // thread by `enter`, deactivated exactly once.
        unsafe { DeactivateActCtx(0, self.0) };
    }
}

fn load_comctl6() -> Result<Comctl6, String> {
    let mut dir = [0u16; 512];
    // SAFETY: the buffer is valid for `dir.len()` UTF-16 units.
    let len = unsafe { GetSystemDirectoryW(dir.as_mut_ptr(), dir.len() as u32) } as usize;
    if len == 0 || len >= dir.len() {
        return Err("cannot locate the Windows system directory".into());
    }
    let system_dir = String::from_utf16_lossy(&dir[..len]);
    let assembly_dir = wide(&system_dir);
    let source = wide(&format!("{system_dir}\\shell32.dll"));
    let request = ACTCTXW {
        cbSize: size_of::<ACTCTXW>() as u32,
        dwFlags: ACTCTX_FLAG_RESOURCE_NAME_VALID | ACTCTX_FLAG_ASSEMBLY_DIRECTORY_VALID,
        lpSource: source.as_ptr(),
        lpAssemblyDirectory: assembly_dir.as_ptr(),
        lpResourceName: ptr::without_provenance::<u16>(SHELL32_COMCTL6_MANIFEST),
        ..ACTCTXW::default()
    };
    // SAFETY: `request` and the strings it points to outlive the call.
    let context = unsafe { CreateActCtxW(&raw const request) };
    if context == INVALID_HANDLE_VALUE {
        // SAFETY: no arguments.
        let code = unsafe { GetLastError() };
        return Err(format!(
            "cannot activate Common Controls v6 (CreateActCtxW error {code})"
        ));
    }
    let _active = Activation::enter(context)?;
    let library = wide("comctl32.dll");
    // SAFETY: NUL-terminated name; loaded under the v6 context, so the
    // side-by-side comctl32 v6 is the module returned. Never freed.
    let module = unsafe { LoadLibraryW(library.as_ptr()) };
    if module.is_null() {
        return Err("cannot load comctl32.dll v6".into());
    }
    // SAFETY: `module` is loaded; the name is a NUL-terminated ANSI string.
    let entry = unsafe { GetProcAddress(module, c"TaskDialogIndirect".as_ptr().cast()) }
        .ok_or_else(|| "comctl32.dll has no TaskDialogIndirect".to_owned())?;
    // SAFETY: `TaskDialogIndirect` has exactly this signature (CommCtrl.h).
    let task_dialog = unsafe {
        std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirectFn>(entry)
    };
    Ok(Comctl6 {
        context: context as isize,
        task_dialog,
    })
}

fn comctl6() -> Result<&'static Comctl6, String> {
    COMCTL6
        .get_or_init(load_comctl6)
        .as_ref()
        .map_err(Clone::clone)
}

/// Command id → response index.
fn response_for(spec: &MessageSpec, command: i32) -> usize {
    match &spec.buttons {
        Buttons::Custom(labels) => usize::try_from(command - CUSTOM_BUTTON_BASE)
            .ok()
            .filter(|index| *index < labels.len())
            .unwrap_or(spec.cancel_id),
        Buttons::Ok => 0,
        Buttons::OkCancel => match command {
            IDOK => 0,
            IDCANCEL => 1,
            _ => spec.cancel_id,
        },
    }
}

/// Response index → command id of the default button.
fn default_command(spec: &MessageSpec) -> i32 {
    match &spec.buttons {
        Buttons::Custom(_) => {
            CUSTOM_BUTTON_BASE + i32::try_from(spec.default_id).unwrap_or_default()
        },
        Buttons::OkCancel if spec.default_id == 1 => IDCANCEL,
        Buttons::Ok | Buttons::OkCancel => IDOK,
    }
}

pub(super) fn show(spec: &MessageSpec) -> Result<MessageOutcome, String> {
    let comctl = comctl6()?;

    let title = wide(&spec.title);
    let message = wide(&spec.message);
    let detail = spec.detail.as_deref().map(wide);
    let checkbox = spec.checkbox_label.as_deref().map(wide);
    let labels: Vec<Vec<u16>> = match &spec.buttons {
        Buttons::Custom(labels) => labels.iter().map(|label| wide(label)).collect(),
        Buttons::Ok | Buttons::OkCancel => Vec::new(),
    };
    let buttons: Vec<TASKDIALOG_BUTTON> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| TASKDIALOG_BUTTON {
            nButtonID: CUSTOM_BUTTON_BASE + i32::try_from(index).unwrap_or_default(),
            pszButtonText: label.as_ptr(),
        })
        .collect();
    let common_buttons = match spec.buttons {
        Buttons::Ok => TDCBF_OK_BUTTON,
        Buttons::OkCancel => TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON,
        Buttons::Custom(_) => 0,
    };

    let owner: HWND = if spec.parent {
        owner_windows::foreground_window()
    } else {
        ptr::null_mut()
    };
    let mut flags = TDF_ALLOW_DIALOG_CANCELLATION;
    if !owner.is_null() {
        flags |= TDF_POSITION_RELATIVE_TO_WINDOW;
    }
    if spec.checkbox_checked {
        flags |= TDF_VERIFICATION_FLAG_CHECKED;
    }
    let icon = match spec.kind {
        Kind::None => TASKDIALOGCONFIG_0 {
            pszMainIcon: ptr::null(),
        },
        Kind::Info => TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_INFORMATION_ICON,
        },
        Kind::Warning => TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_WARNING_ICON,
        },
        Kind::Error => TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_ERROR_ICON,
        },
        Kind::Question => {
            flags |= TDF_USE_HICON_MAIN;
            // SAFETY: stock system icon; a null module selects the
            // predefined icons. Shared icons are never destroyed.
            TASKDIALOGCONFIG_0 {
                hMainIcon: unsafe { LoadIconW(ptr::null_mut(), IDI_QUESTION) },
            }
        },
    };
    let text_or_null =
        |text: Option<&Vec<u16>>| -> PCWSTR { text.map_or(ptr::null(), Vec::as_ptr) };

    let config = TASKDIALOGCONFIG {
        cbSize: size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        dwFlags: flags,
        dwCommonButtons: common_buttons,
        pszWindowTitle: title.as_ptr(),
        Anonymous1: icon,
        pszMainInstruction: message.as_ptr(),
        pszContent: text_or_null(detail.as_ref()),
        cButtons: buttons.len() as u32,
        pButtons: if buttons.is_empty() {
            ptr::null()
        } else {
            buttons.as_ptr()
        },
        nDefaultButton: default_command(spec),
        pszVerificationText: text_or_null(checkbox.as_ref()),
        ..TASKDIALOGCONFIG::default()
    };

    let _active = Activation::enter(comctl.context as *mut c_void)?;
    let mut command = 0i32;
    let mut checked: BOOL = 0;
    // SAFETY: `config` and every string / button array it points to live
    // until the call returns; the out pointers are valid. The v6 context is
    // active on this thread for the dialog's lifetime.
    let hr = unsafe {
        (comctl.task_dialog)(
            &raw const config,
            &raw mut command,
            ptr::null_mut(),
            if checkbox.is_some() {
                &raw mut checked
            } else {
                ptr::null_mut()
            },
        )
    };
    if hr < 0 {
        return Err(format!("TaskDialogIndirect failed (HRESULT {hr:#010x})"));
    }
    Ok(MessageOutcome {
        response: response_for(spec, command),
        checkbox_checked: checked != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::dialog::message::MessageArgs;

    fn spec(buttons: Option<Vec<&str>>, preset: Buttons) -> MessageSpec {
        MessageSpec::parse(
            MessageArgs {
                message: "m".into(),
                buttons: buttons.map(|b| b.into_iter().map(str::to_owned).collect()),
                ..MessageArgs::default()
            },
            preset,
            "T",
        )
        .expect("valid")
    }

    #[test]
    fn maps_commands_to_response_indexes() {
        let custom = spec(Some(vec!["Save", "Discard", "Cancel"]), Buttons::Ok);
        assert_eq!(response_for(&custom, CUSTOM_BUTTON_BASE + 1), 1);
        // Esc / close box → the cancel button.
        assert_eq!(response_for(&custom, IDCANCEL), 2);
        assert_eq!(default_command(&custom), CUSTOM_BUTTON_BASE);

        let confirm = spec(None, Buttons::OkCancel);
        assert_eq!(response_for(&confirm, IDOK), 0);
        assert_eq!(response_for(&confirm, IDCANCEL), 1);
        assert_eq!(response_for(&spec(None, Buttons::Ok), IDCANCEL), 0);
    }

    #[test]
    fn resolves_task_dialog_under_the_v6_context() {
        // Loading only (no dialog shown): proves the shell32 manifest trick
        // works on this Windows.
        assert!(comctl6().is_ok(), "{:?}", comctl6().err());
    }
}
