//! Recent documents: the app's jump list "Recent" category on Windows and
//! the "Open Recent" menu list on macOS. Linux has no per-app list.

use std::path::Path;

#[cfg(target_os = "windows")]
mod imp {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};

    use windows_sys::{
        core::{GUID, HRESULT},
        Win32::{
            Foundation::RPC_E_CHANGED_MODE,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{ApplicationDestinations, SHAddToRecentDocs, SHARD_PATHW},
        },
    };

    /// `IID_IApplicationDestinations` (ShObjIdl_core.h).
    const IID_APPLICATION_DESTINATIONS: GUID =
        GUID::from_u128(0x12337d35_94c6_48a0_bce7_6a9c69d4d600);

    /// Vtable of `IApplicationDestinations` (IUnknown, then its 3 methods).
    #[repr(C)]
    struct DestinationsVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(this: *mut c_void) -> u32,
        set_app_id: usize,
        remove_destination: usize,
        remove_all_destinations: unsafe extern "system" fn(this: *mut c_void) -> HRESULT,
    }

    pub(super) fn add(path: &Path) -> Result<(), String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: NUL-terminated UTF-16 path, alive for the call.
        unsafe { SHAddToRecentDocs(SHARD_PATHW as u32, wide.as_ptr().cast()) };
        Ok(())
    }

    /// COM apartment for this call; uninitialized on drop when this call
    /// initialized it.
    struct Apartment(bool);

    impl Apartment {
        fn enter() -> Self {
            // SAFETY: reserved pointer must be null.
            let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
            // S_OK / S_FALSE need a matching uninit; RPC_E_CHANGED_MODE means
            // the thread already has a (multi-threaded) apartment.
            Self(hr >= 0 && hr != RPC_E_CHANGED_MODE)
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful `CoInitializeEx` above.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// Remove this app's recent (and frequent) jump-list destinations. The
    /// Explorer-wide recent items of other apps are left alone.
    pub(super) fn clear() -> Result<(), String> {
        let _apartment = Apartment::enter();
        let mut object: *mut c_void = ptr::null_mut();
        let clsid = ApplicationDestinations;
        let iid = IID_APPLICATION_DESTINATIONS;
        // SAFETY: valid CLSID / IID pointers and out pointer.
        let hr = unsafe {
            CoCreateInstance(
                &raw const clsid,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &raw const iid,
                &raw mut object,
            )
        };
        if hr < 0 || object.is_null() {
            return Err(format!("cannot open the jump list (HRESULT {hr:#010x})"));
        }
        // SAFETY: `object` is a live `IApplicationDestinations`; its first
        // field is the vtable pointer, laid out as `DestinationsVtbl`.
        let vtbl = unsafe { &**object.cast::<*const DestinationsVtbl>() };
        // SAFETY: `object` is live; each method is called with it as `this`,
        // and `release` drops the reference `CoCreateInstance` handed out.
        let hr = unsafe {
            let hr = (vtbl.remove_all_destinations)(object);
            (vtbl.release)(object);
            hr
        };
        if hr < 0 {
            return Err(format!("cannot clear the jump list (HRESULT {hr:#010x})"));
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::Path;

    use dispatch2::run_on_main;
    use objc2_app_kit::NSDocumentController;
    use objc2_foundation::{NSString, NSURL};

    pub(super) fn add(path: &Path) -> Result<(), String> {
        let path = path
            .to_str()
            .ok_or_else(|| "path is not valid UTF-8".to_owned())?
            .to_owned();
        run_on_main(move |mtm| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
            NSDocumentController::sharedDocumentController(mtm).noteNewRecentDocumentURL(&url);
        });
        Ok(())
    }

    pub(super) fn clear() -> Result<(), String> {
        run_on_main(|mtm| {
            // SAFETY: `clearRecentDocuments:` accepts a nil sender.
            unsafe {
                NSDocumentController::sharedDocumentController(mtm).clearRecentDocuments(None);
            }
        });
        Ok(())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use std::path::Path;

    const UNSUPPORTED: &str = "recent documents are not supported on Linux (no per-app list)";

    pub(super) fn add(_path: &Path) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }

    pub(super) fn clear() -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
}

/// Add `path` (already authorized and resolved) to the OS recent list.
pub(super) fn add(path: &Path) -> Result<(), String> {
    imp::add(path)
}

/// Clear this app's recent list.
pub(super) fn clear() -> Result<(), String> {
    imp::clear()
}
