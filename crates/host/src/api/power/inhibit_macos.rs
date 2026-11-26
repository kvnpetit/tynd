//! macOS keep-awake through IOKit power assertions (`IOPMAssertionCreateWithName`).
//! The IOKit / CoreFoundation symbols are declared by hand: only three calls
//! are needed, which does not justify a binding crate.

use std::ffi::{c_char, c_void, CStr};

type CfStringRef = *const c_void;
type IoReturn = i32;

const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const K_IO_RETURN_SUCCESS: IoReturn = 0;

const SYSTEM_SLEEP_ASSERTION: &CStr = c"PreventUserIdleSystemSleep";
const DISPLAY_SLEEP_ASSERTION: &CStr = c"PreventUserIdleDisplaySleep";
const ASSERTION_NAME: &CStr = c"Tynd app requested keep-awake";

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CfStringRef,
        assertion_level: u32,
        assertion_name: CfStringRef,
        assertion_id: *mut u32,
    ) -> IoReturn;
    fn IOPMAssertionRelease(assertion_id: u32) -> IoReturn;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        allocator: *const c_void,
        c_str: *const c_char,
        encoding: u32,
    ) -> CfStringRef;
    fn CFRelease(cf: *const c_void);
}

/// Owned `CFString` released on drop.
struct CfString(CfStringRef);

impl CfString {
    fn new(text: &CStr) -> Result<Self, String> {
        // SAFETY: `text` is NUL-terminated; a null allocator selects the
        // default CoreFoundation allocator. The result is owned (Create rule).
        let raw = unsafe {
            CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        };
        if raw.is_null() {
            Err("power.preventSleep: CFStringCreateWithCString failed".into())
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for CfString {
    fn drop(&mut self) {
        // SAFETY: the string was created with a Create function and is
        // released exactly once.
        unsafe { CFRelease(self.0) };
    }
}

pub(super) struct Handle {
    assertion_id: u32,
}

pub(super) fn acquire(display: bool) -> Result<Handle, String> {
    let kind = CfString::new(if display {
        DISPLAY_SLEEP_ASSERTION
    } else {
        SYSTEM_SLEEP_ASSERTION
    })?;
    let name = CfString::new(ASSERTION_NAME)?;
    let mut assertion_id = 0_u32;
    // SAFETY: both strings are live for the call (IOKit retains what it
    // needs) and `assertion_id` is a valid out pointer.
    let status = unsafe {
        IOPMAssertionCreateWithName(
            kind.0,
            K_IOPM_ASSERTION_LEVEL_ON,
            name.0,
            &raw mut assertion_id,
        )
    };
    if status == K_IO_RETURN_SUCCESS {
        Ok(Handle { assertion_id })
    } else {
        Err(format!(
            "power.preventSleep: IOPMAssertionCreateWithName failed (0x{status:x})"
        ))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the assertion was created by `acquire` and is released once.
        unsafe { IOPMAssertionRelease(self.assertion_id) };
    }
}
