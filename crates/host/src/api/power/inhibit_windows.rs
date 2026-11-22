//! Windows keep-awake through power requests. Unlike `SetThreadExecutionState`
//! (which binds to the calling thread), a request handle is process-wide, so
//! requests can be made and released from any dispatch thread.

use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Power::{
            PowerClearRequest, PowerCreateRequest, PowerRequestDisplayRequired,
            PowerRequestSystemRequired, PowerSetRequest,
        },
        Threading::{POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0},
    },
};

/// `POWER_REQUEST_CONTEXT_VERSION` (winnt.h); windows-sys does not export it.
const POWER_REQUEST_CONTEXT_VERSION: u32 = 0;

pub(super) struct Handle {
    request: HANDLE,
    display: bool,
}

// SAFETY: a power request handle is a kernel object handle usable from any
// thread; `Handle` is the sole owner and closes it once on drop.
unsafe impl Send for Handle {}

pub(super) fn acquire(display: bool) -> Result<Handle, String> {
    let mut reason: Vec<u16> = "Tynd app requested keep-awake\0".encode_utf16().collect();
    let context = REASON_CONTEXT {
        Version: POWER_REQUEST_CONTEXT_VERSION,
        Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
        Reason: REASON_CONTEXT_0 {
            SimpleReasonString: reason.as_mut_ptr(),
        },
    };
    // SAFETY: `context` and the string it points to outlive the call; the
    // returned handle is owned by `Handle` below.
    let request = unsafe { PowerCreateRequest(&raw const context) };
    if request.is_null() || request == INVALID_HANDLE_VALUE {
        // SAFETY: plain thread-local error query.
        let code = unsafe { GetLastError() };
        return Err(format!(
            "power.preventSleep: PowerCreateRequest failed ({code})"
        ));
    }
    // From here `Drop` closes the handle on every early return.
    let handle = Handle {
        request,
        display: false,
    };

    // SAFETY: `request` is a live power request handle.
    if unsafe { PowerSetRequest(request, PowerRequestSystemRequired) } == 0 {
        let code = unsafe { GetLastError() };
        return Err(format!(
            "power.preventSleep: PowerSetRequest failed ({code})"
        ));
    }
    let mut handle = handle;
    if display {
        // SAFETY: same live handle as above.
        if unsafe { PowerSetRequest(request, PowerRequestDisplayRequired) } == 0 {
            let code = unsafe { GetLastError() };
            return Err(format!(
                "power.preventSleep: PowerSetRequest(display) failed ({code})"
            ));
        }
        handle.display = true;
    }
    Ok(handle)
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: `request` is owned by this handle and closed exactly once.
        // Clearing a request type that was never set fails harmlessly.
        unsafe {
            PowerClearRequest(self.request, PowerRequestSystemRequired);
            if self.display {
                PowerClearRequest(self.request, PowerRequestDisplayRequired);
            }
            CloseHandle(self.request);
        }
    }
}
