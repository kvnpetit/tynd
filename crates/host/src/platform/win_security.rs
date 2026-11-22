//! Owner-only DACL builder for Windows named pipes.
//!
//! Default `interprocess::ListenerOptions` creates pipes with a
//! permissive DACL — the interactive group ends up with read/write,
//! which means any local non-admin user can connect and inject argv
//! through `singleInstance` or a deep-link URL through
//! `api::deep_link::forwarding`. Both surfaces drive privileged code in
//! the existing instance, so a permissive DACL is a real privilege
//! boundary breach on multi-user boxes.
//!
//! `current_user_sd()` returns a [`SecurityDescriptor`] whose DACL
//! grants Generic-All only to the current user's SID. Pass it to
//! `ListenerOptions::security_descriptor(...)` (via the
//! `ListenerOptionsExt` trait) on Windows builds.

use std::{ffi::c_void, io, ptr};

use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use widestring::U16CString;
use windows_sys::Win32::{
    Foundation::{CloseHandle, LocalFree, HANDLE},
    Security::{
        Authorization::ConvertSidToStringSidW, EqualSid, GetTokenInformation, TokenUser, PSID,
        TOKEN_QUERY, TOKEN_USER,
    },
    System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    },
};

/// Build a `SecurityDescriptor` whose DACL grants Generic-All only to
/// the running user. The descriptor is also given an explicit owner +
/// group of the same SID so SDDL `OW`/`OG` references resolve.
///
/// Errors propagate the underlying Win32 error.
pub(crate) fn current_user_sd() -> io::Result<SecurityDescriptor> {
    let sid = current_user_sid_string()?;
    // SDDL: protected DACL, owner+group set to the user, single ACE
    // granting Generic-All to the same SID.
    let sddl = format!("O:{sid}G:{sid}D:P(A;;GA;;;{sid})");
    let sddl_w = U16CString::from_str(&sddl)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    SecurityDescriptor::deserialize(&sddl_w)
}

/// SID of the running user in string form (`S-1-5-21-...`).
pub(crate) fn current_user_sid_string() -> io::Result<String> {
    let token = open_process_token(unsafe { GetCurrentProcess() })?;
    let buf = token_user_buffer(token.0)?;
    let sid_ptr = token_user_sid(&buf)?;

    let mut sid_str_w: *mut u16 = ptr::null_mut();
    let ok = unsafe { ConvertSidToStringSidW(sid_ptr, &raw mut sid_str_w) };
    if ok == 0 || sid_str_w.is_null() {
        return Err(io::Error::last_os_error());
    }
    let sid_string = unsafe { wide_ptr_to_string(sid_str_w) };
    unsafe {
        LocalFree(sid_str_w.cast::<c_void>());
    }
    Ok(sid_string)
}

/// True when process `pid` runs as the same user as this process. Used by
/// forwarding clients to verify the pipe server before writing argv / URLs
/// (OAuth codes) to it: a pipe squatter running as another account is refused.
pub(crate) fn process_is_current_user(pid: u32) -> io::Result<bool> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = TokenGuard(process);
    let peer_token = open_process_token(process.0)?;
    let peer = token_user_buffer(peer_token.0)?;
    let own_token = open_process_token(unsafe { GetCurrentProcess() })?;
    let own = token_user_buffer(own_token.0)?;
    let equal = unsafe { EqualSid(token_user_sid(&peer)?, token_user_sid(&own)?) };
    Ok(equal != 0)
}

fn open_process_token(process: HANDLE) -> io::Result<TokenGuard> {
    let mut token: HANDLE = ptr::null_mut();
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(TokenGuard(token))
}

/// `TOKEN_USER` blob for `token`, backed by a `u64` buffer so the cast to
/// `TOKEN_USER` stays well-aligned on every supported target.
fn token_user_buffer(token: HANDLE) -> io::Result<Vec<u64>> {
    let mut size: u32 = 0;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &raw mut size);
    }
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let words = (size as usize).div_ceil(size_of::<u64>());
    let mut buf: Vec<u64> = vec![0u64; words];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast::<c_void>(),
            size,
            &raw mut size,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buf)
}

fn token_user_sid(buf: &[u64]) -> io::Result<PSID> {
    let token_user = unsafe { &*(buf.as_ptr().cast::<TOKEN_USER>()) };
    let sid_ptr = token_user.User.Sid;
    if sid_ptr.is_null() {
        return Err(io::Error::other("null user SID"));
    }
    Ok(sid_ptr)
}

unsafe fn wide_ptr_to_string(p: *const u16) -> String {
    let mut len = 0;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(p, len) };
    String::from_utf16_lossy(slice)
}

struct TokenGuard(HANDLE);
impl Drop for TokenGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
