//! macOS-side URL scheme registration via Launch Services + an
//! `NSAppleEventManager` handler for runtime `GURL` events.
//!
//! ## What works at runtime
//!
//! * `register` / `unregister` flip the *default* handler via
//!   `LSSetDefaultHandlerForURLScheme(scheme, bundleId)`.
//! * `is_registered` reads `LSCopyDefaultHandlerForURLScheme` and compares against this binary's
//!   bundle id.
//! * `install_apple_event_handler` wires a Carbon-style Apple Event handler under
//!   `kInternetEventClass` / `kAEGetURL` so URLs that arrive after the app started land on
//!   `deep-link:url`.
//!
//! ## What must be done at build time (out of scope)
//!
//! `Info.plist` must declare `CFBundleURLTypes` with the scheme so
//! Launch Services *knows* this app can handle it before the runtime
//! call below is allowed to flip the default. The Tynd CLI handles
//! that injection from `tynd.config.ts`'s `bundle.macos.urlSchemes`
//! array; this module only owns the runtime side.

#![cfg(target_os = "macos")]

use std::{
    ffi::{c_void, CString},
    sync::OnceLock,
};

// ── Core Foundation FFI ────────────────────────────────────────────────────

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFAllocatorRef = *const c_void;
type CFIndex = isize;
type CFStringEncoding = u32;
type Boolean = u8;
type OSStatus = i32;
type OSType = u32;
type OSErr = i16;

const K_CF_STRING_ENCODING_UTF8: CFStringEncoding = 0x0800_0100;
const K_CF_ALLOCATOR_DEFAULT: CFAllocatorRef = std::ptr::null();

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const i8,
        encoding: CFStringEncoding,
    ) -> CFStringRef;
    fn CFStringGetCString(
        the_string: CFStringRef,
        buffer: *mut i8,
        buffer_size: CFIndex,
        encoding: CFStringEncoding,
    ) -> Boolean;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: CFStringEncoding) -> CFIndex;
    fn CFStringGetLength(the_string: CFStringRef) -> CFIndex;
    fn CFRelease(cf: CFTypeRef);
}

// ── Launch Services ────────────────────────────────────────────────────────

#[link(name = "CoreServices", kind = "framework")]
extern "C" {
    fn LSSetDefaultHandlerForURLScheme(
        in_url_scheme: CFStringRef,
        in_handler_bundle_id: CFStringRef,
    ) -> OSStatus;
    fn LSCopyDefaultHandlerForURLScheme(in_url_scheme: CFStringRef) -> CFStringRef;
}

// ── Carbon Apple Event API for runtime URL events ──────────────────────────

const K_INTERNET_EVENT_CLASS: OSType = u32::from_be_bytes(*b"GURL");
const K_AE_GET_URL: OSType = u32::from_be_bytes(*b"GURL");
const KEY_DIRECT_OBJECT: OSType = u32::from_be_bytes(*b"----");
const TYPE_UTF8_TEXT: OSType = u32::from_be_bytes(*b"utf8");

type AEEventHandlerProcPtr =
    unsafe extern "C" fn(event: *const c_void, reply: *mut c_void, refcon: isize) -> OSErr;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn AEInstallEventHandler(
        event_class: OSType,
        event_id: OSType,
        handler: AEEventHandlerProcPtr,
        refcon: isize,
        is_sys_handler: Boolean,
    ) -> OSStatus;
    fn AEGetParamPtr(
        event: *const c_void,
        key: OSType,
        desired_type: OSType,
        actual_type: *mut OSType,
        out_buf: *mut c_void,
        max_size: isize,
        actual_size: *mut isize,
    ) -> OSStatus;
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn cfstring_from_str(s: &str) -> Option<CFStringRef> {
    let c = CString::new(s).ok()?;
    let cf = unsafe {
        CFStringCreateWithCString(
            K_CF_ALLOCATOR_DEFAULT,
            c.as_ptr(),
            K_CF_STRING_ENCODING_UTF8,
        )
    };
    if cf.is_null() {
        None
    } else {
        Some(cf)
    }
}

fn cfstring_to_string(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let len = unsafe { CFStringGetLength(s) };
    let cap = unsafe { CFStringGetMaximumSizeForEncoding(len, K_CF_STRING_ENCODING_UTF8) } + 1;
    let mut buf = vec![0i8; cap as usize];
    let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), cap, K_CF_STRING_ENCODING_UTF8) };
    if ok == 0 {
        return None;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    cstr.to_str().ok().map(str::to_string)
}

fn current_bundle_id() -> String {
    if let Ok(id) = std::env::var("__CFBundleIdentifier") {
        return id;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "com.tynd.app".into())
}

// ── Public surface ─────────────────────────────────────────────────────────

pub(super) fn register(scheme: &str) -> Result<(), String> {
    let bundle = current_bundle_id();
    let scheme_cf =
        cfstring_from_str(scheme).ok_or_else(|| "scheme contains a NUL byte".to_string())?;
    let bundle_cf =
        cfstring_from_str(&bundle).ok_or_else(|| "bundle id contains a NUL byte".to_string())?;
    let status = unsafe { LSSetDefaultHandlerForURLScheme(scheme_cf, bundle_cf) };
    unsafe {
        CFRelease(scheme_cf);
        CFRelease(bundle_cf);
    }
    if status != 0 {
        return Err(format!("LSSetDefaultHandlerForURLScheme returned {status}"));
    }
    Ok(())
}

pub(super) fn unregister(scheme: &str) -> Result<(), String> {
    // Pass NULL bundle id so Launch Services treats unregister as 'clear handler'.
    let scheme_cf =
        cfstring_from_str(scheme).ok_or_else(|| "scheme contains a NUL byte".to_string())?;
    let status = unsafe { LSSetDefaultHandlerForURLScheme(scheme_cf, std::ptr::null()) };
    unsafe {
        CFRelease(scheme_cf);
    }
    if status != 0 {
        return Err(format!("LSSetDefaultHandlerForURLScheme returned {status}"));
    }
    Ok(())
}

pub(super) fn is_registered(scheme: &str) -> Result<bool, String> {
    let scheme_cf =
        cfstring_from_str(scheme).ok_or_else(|| "scheme contains a NUL byte".to_string())?;
    let cur = unsafe { LSCopyDefaultHandlerForURLScheme(scheme_cf) };
    unsafe { CFRelease(scheme_cf) };
    if cur.is_null() {
        return Ok(false);
    }
    // `cur` is a Copy-rule reference (+1 retain); release it before
    // comparing so no path leaks one CFString per call.
    let handler = cfstring_to_string(cur);
    unsafe { CFRelease(cur) };
    let handler = handler.ok_or_else(|| "default handler is not valid UTF-8".to_string())?;
    Ok(bundle_ids_match(&handler, &current_bundle_id()))
}

/// Launch Services treats bundle identifiers case-insensitively, and the
/// handler it reports may differ in case from `CFBundleIdentifier`.
fn bundle_ids_match(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

// ── Apple Event handler — runtime `GURL` events ─────────────────────────────

/// Marker that the Apple Event handler has been installed for this
/// process. Idempotent — `AEInstallEventHandler` returns an error on
/// duplicate installs which we'd otherwise log noisily.
static HANDLER_INSTALLED: OnceLock<()> = OnceLock::new();

unsafe extern "C" fn apple_event_handler(
    event: *const c_void,
    _reply: *mut c_void,
    _refcon: isize,
) -> OSErr {
    // Stack buffer covers the common case (short scheme://host/path); OAuth
    // callbacks with PKCE/JWT in the fragment can exceed 4 KiB, so we
    // re-issue `AEGetParamPtr` against a heap allocation when Carbon reports
    // a larger payload than we requested.
    let mut buf = [0u8; 4096];
    let mut actual_type: OSType = 0;
    let mut actual_size: isize = 0;
    let status = unsafe {
        AEGetParamPtr(
            event,
            KEY_DIRECT_OBJECT,
            TYPE_UTF8_TEXT,
            &mut actual_type,
            buf.as_mut_ptr().cast::<c_void>(),
            buf.len() as isize,
            &mut actual_size,
        )
    };
    if status != 0 || actual_size <= 0 {
        return 0;
    }
    let needed = actual_size as usize;
    let url = if needed <= buf.len() {
        match std::str::from_utf8(&buf[..needed]) {
            Ok(s) => s.to_string(),
            Err(_) => return 0,
        }
    } else {
        tracing::warn!(
            needed_bytes = needed,
            stack_capacity = buf.len(),
            "apple_event_handler: URL exceeded stack buffer; retrying on heap"
        );
        let mut heap = vec![0u8; needed];
        let mut heap_actual_type: OSType = 0;
        let mut heap_actual_size: isize = 0;
        let heap_status = unsafe {
            AEGetParamPtr(
                event,
                KEY_DIRECT_OBJECT,
                TYPE_UTF8_TEXT,
                &mut heap_actual_type,
                heap.as_mut_ptr().cast::<c_void>(),
                heap.len() as isize,
                &mut heap_actual_size,
            )
        };
        if heap_status != 0 || heap_actual_size <= 0 {
            return 0;
        }
        let got = (heap_actual_size as usize).min(heap.len());
        match std::str::from_utf8(&heap[..got]) {
            Ok(s) => s.to_string(),
            Err(_) => return 0,
        }
    };
    // FFI-safe panic boundary: panicking across an `extern "C"` callback
    // back into Carbon's stack-unwinding (Objective-C exception model)
    // is undefined behaviour. `catch_unwind` keeps any host panic
    // contained to Rust frames; if `emit_url` panics we just drop the
    // URL and return success — Carbon would otherwise treat us as
    // having corrupted its event loop.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::emit_url(&url);
    }));
    0
}

pub(super) fn install_apple_event_handler() {
    // Install first; only flip the latch on noErr so a transient Carbon
    // failure leaves the door open for a later retry. The previous
    // unconditional `set()` left the OnceLock primed even on failure,
    // wedging deep-link delivery for the rest of the process lifetime.
    if HANDLER_INSTALLED.get().is_some() {
        return;
    }
    let status = unsafe {
        AEInstallEventHandler(
            K_INTERNET_EVENT_CLASS,
            K_AE_GET_URL,
            apple_event_handler,
            0,
            0,
        )
    };
    if status != 0 {
        tracing::warn!(
            target: "tynd::deep_link",
            "AEInstallEventHandler failed (OSStatus={status}); deep-link URL events will not arrive"
        );
        return;
    }
    let _ = HANDLER_INSTALLED.set(());
}

#[cfg(test)]
mod tests {
    use super::bundle_ids_match;

    #[test]
    fn bundle_ids_compare_ascii_case_insensitively() {
        assert!(bundle_ids_match("com.example.MyApp", "com.example.myapp"));
        assert!(bundle_ids_match("com.example.app", "com.example.app"));
        assert!(!bundle_ids_match("com.example.app", "com.example.other"));
    }
}
