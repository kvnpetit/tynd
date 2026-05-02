//! macOS-side URL scheme registration via Launch Services + an
//! `NSAppleEventManager` handler for runtime `GURL` events.
//!
//! ## What works at runtime
//!
//! * `register` / `unregister` flip the *default* handler via
//!   `LSSetDefaultHandlerForURLScheme(scheme, bundleId)`.
//! * `is_registered` reads `LSCopyDefaultHandlerForURLScheme` and
//!   compares against this binary's bundle id.
//! * `install_apple_event_handler` wires a Carbon-style Apple Event
//!   handler under `kInternetEventClass` / `kAEGetURL` so URLs that
//!   arrive after the app started land on `deep-link:url`.
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
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> Boolean;
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
        CFStringCreateWithCString(K_CF_ALLOCATOR_DEFAULT, c.as_ptr(), K_CF_STRING_ENCODING_UTF8)
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
    // Pass `NULL` (a null `CFStringRef`) as the bundle id — Launch
    // Services treats that as "clear the handler" and falls back to
    // whatever else has the scheme in its `CFBundleURLTypes`. The
    // previous implementation set `com.tynd.unregistered` (a bundle id
    // that doesn't exist) which polluted LaunchServices' database with
    // a phantom entry per scheme — visible in `lsregister -dump` until
    // the next `lsregister -kill -r`.
    //
    // If a future macOS release rejects NULL here (it currently
    // doesn't — Apple's own apps use the same trick), we'd need to
    // synthesize a "no-op" handler at install time and hand the scheme
    // back to it. For now NULL is the cleanest unregister.
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
    let bundle = current_bundle_id();
    let bundle_cf =
        cfstring_from_str(&bundle).ok_or_else(|| "bundle id contains a NUL byte".to_string())?;
    let eq = unsafe { CFEqual(cur as CFTypeRef, bundle_cf as CFTypeRef) };
    unsafe {
        CFRelease(cur);
        CFRelease(bundle_cf);
    }
    Ok(eq != 0)
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
    let len = (actual_size as usize).min(buf.len());
    let url = match std::str::from_utf8(&buf[..len]) {
        Ok(s) => s.to_string(),
        Err(_) => return 0,
    };
    super::emit_url(&url);
    0
}

pub(super) fn install_apple_event_handler() {
    if HANDLER_INSTALLED.set(()).is_err() {
        return;
    }
    let _ = unsafe {
        AEInstallEventHandler(
            K_INTERNET_EVENT_CLASS,
            K_AE_GET_URL,
            apple_event_handler,
            0,
            0,
        )
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cfstring_round_trip_preserves_utf8() {
        let cf = cfstring_from_str("héllo").unwrap();
        let back = cfstring_to_string(cf).unwrap();
        unsafe { CFRelease(cf) };
        assert_eq!(back, "héllo");
    }

    #[test]
    fn current_bundle_id_returns_some_string() {
        let id = current_bundle_id();
        assert!(!id.is_empty());
    }
}
