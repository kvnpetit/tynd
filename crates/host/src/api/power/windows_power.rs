//! Hidden top-level window + private message pump for
//! `WM_POWERBROADCAST`, `WM_WTSSESSION_CHANGE` and `WM_ENDSESSION`. The
//! window is top-level (never shown, `WS_EX_TOOLWINDOW` keeps it off the
//! taskbar) because message-only windows receive no broadcast messages,
//! which is how Windows delivers suspend/resume and session end. The
//! watcher thread owns the HWND and stops on `WM_QUIT` during central host
//! shutdown.
use std::sync::{
    atomic::{AtomicU8, Ordering},
    mpsc,
};

use serde_json::json;
use windows_sys::core::GUID;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Power::{
            RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
            POWERBROADCAST_SETTING,
        },
        RemoteDesktop::{
            WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
            NOTIFY_FOR_THIS_SESSION,
        },
        Threading::GetCurrentThreadId,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        PeekMessageW, PostThreadMessageW, RegisterClassExW, TranslateMessage,
        DEVICE_NOTIFY_WINDOW_HANDLE, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
        PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, PM_NOREMOVE, WM_ENDSESSION, WM_POWERBROADCAST,
        WM_QUIT, WM_WTSSESSION_CHANGE, WNDCLASSEXW, WS_EX_TOOLWINDOW, WTS_SESSION_LOCK,
        WTS_SESSION_UNLOCK,
    },
};

use super::{join_workers, MONITOR_START_TIMEOUT};
use crate::ipc::events;

/// UTF-16 NUL-terminated. `'static` so it outlives the window class.
const CLASS_NAME: &[u16] = &[
    b'T' as u16,
    b'y' as u16,
    b'n' as u16,
    b'd' as u16,
    b'P' as u16,
    b'o' as u16,
    b'w' as u16,
    b'e' as u16,
    b'r' as u16,
    b'W' as u16,
    b'a' as u16,
    b't' as u16,
    b'c' as u16,
    b'h' as u16,
    b'e' as u16,
    b'r' as u16,
    0,
];

/// `GUID_ACDC_POWER_SOURCE` (WinNT.h): data is a DWORD, 0 = AC,
/// 1 = battery, 2 = short-term source (UPS).
static GUID_ACDC_POWER_SOURCE: GUID = GUID::from_u128(0x5d3e_9a59_e9d5_4b00_a6bd_ff34_ff51_6548);

/// Last reported power source, `SOURCE_UNKNOWN` until the first
/// notification. Windows sends one right after registration; it only seeds
/// the value so `power:source-change` fires on real transitions.
static LAST_SOURCE: AtomicU8 = AtomicU8::new(SOURCE_UNKNOWN);
const SOURCE_UNKNOWN: u8 = u8::MAX;

pub(super) struct Monitor {
    thread_id: u32,
    join: std::thread::JoinHandle<()>,
}

impl Monitor {
    pub(super) fn shutdown(self) {
        stop_thread(self.thread_id);
        join_workers(vec![self.join], "power[win]");
    }
}

impl super::MonitorHandle for Monitor {
    fn shutdown(self) {
        Monitor::shutdown(self);
    }

    fn is_alive(&self) -> bool {
        !self.join.is_finished()
    }
}

pub(super) fn spawn_watcher() -> Result<Monitor, String> {
    let (thread_id_tx, thread_id_rx) = mpsc::sync_channel(1);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let join = std::thread::Builder::new()
        .name("tynd-power-win".into())
        .spawn(move || {
            // SAFETY: querying current thread id and touching its message
            // queue have no external preconditions.
            let thread_id = unsafe { GetCurrentThreadId() };
            let mut msg = MSG::default();
            unsafe {
                PeekMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            }
            if thread_id_tx.send(thread_id).is_err() {
                return;
            }

            if let Err(error) = run(&ready_tx) {
                let _ = ready_tx.send(Err(error.clone()));
                crate::tynd_warn!("power[win]: watcher exited: {error}");
            }
        })
        .map_err(|error| format!("power[win]: watcher thread spawn failed: {error}"))?;

    let thread_id = match thread_id_rx.recv_timeout(MONITOR_START_TIMEOUT) {
        Ok(thread_id) => thread_id,
        Err(error) => {
            // Drop receiver first: a late worker observes send failure and
            // exits before creating its hidden window.
            drop(thread_id_rx);
            join_workers(vec![join], "power[win]");
            return Err(format!(
                "power[win]: watcher startup handshake failed: {error}"
            ));
        },
    };

    match ready_rx.recv_timeout(MONITOR_START_TIMEOUT) {
        Ok(Ok(())) => Ok(Monitor { thread_id, join }),
        Ok(Err(error)) => {
            join_workers(vec![join], "power[win]");
            Err(error)
        },
        Err(error) => {
            stop_thread(thread_id);
            join_workers(vec![join], "power[win]");
            Err(format!("power[win]: watcher readiness failed: {error}"))
        },
    }
}

fn stop_thread(thread_id: u32) {
    // SAFETY: watcher creates its message queue before publishing
    // `thread_id`; posting WM_QUIT is documented cross-thread stop.
    let posted = unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) } != 0;
    if !posted {
        crate::tynd_warn!("power[win]: PostThreadMessageW(WM_QUIT) failed");
    }
}

fn run(ready: &mpsc::SyncSender<Result<(), String>>) -> Result<(), String> {
    // SAFETY: `GetModuleHandleW(NULL)` returns current module HINSTANCE.
    let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };

    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: CLASS_NAME.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    // SAFETY: class is fully populated and CLASS_NAME is static. Duplicate
    // registration is tolerated because CreateWindowExW resolves it.
    unsafe { RegisterClassExW(&raw const class) };

    // SAFETY: a top-level window without `WS_VISIBLE` is never shown;
    // `WS_EX_TOOLWINDOW` keeps it out of the taskbar and Alt+Tab.
    let hwnd: HWND = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS_NAME.as_ptr(),
            CLASS_NAME.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("power[win]: CreateWindowExW failed".into());
    }

    // SAFETY: hwnd is owned by this thread and registration is scoped to
    // current user session.
    let session_ok = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;
    if !session_ok {
        crate::tynd_warn!(
            "power[win]: WTSRegisterSessionNotification failed; lock/unlock events disabled. \
             Suspend/resume still work."
        );
    }

    LAST_SOURCE.store(SOURCE_UNKNOWN, Ordering::Release);
    // SAFETY: hwnd is a valid window owned by this thread and the GUID is
    // a static. A zero handle means registration failed.
    let source = unsafe {
        RegisterPowerSettingNotification(
            hwnd,
            &raw const GUID_ACDC_POWER_SOURCE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        )
    };
    if source == 0 {
        crate::tynd_warn!(
            "power[win]: RegisterPowerSettingNotification failed; power-source events disabled"
        );
    }
    let handles = Registrations {
        session: session_ok,
        source,
    };

    if ready.send(Ok(())).is_err() {
        cleanup(hwnd, &handles);
        return Ok(());
    }

    let mut msg = MSG::default();
    let mut pump_failed = false;
    loop {
        // SAFETY: msg points to writable storage owned by this thread.
        let result = unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) };
        if result == 0 {
            break;
        }
        if result == -1 {
            pump_failed = true;
            break;
        }
        // SAFETY: msg was populated by GetMessageW.
        unsafe {
            TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }

    cleanup(hwnd, &handles);
    if pump_failed {
        Err("power[win]: GetMessageW failed".into())
    } else {
        Ok(())
    }
}

/// Notification registrations to undo before the window is destroyed.
struct Registrations {
    session: bool,
    source: HPOWERNOTIFY,
}

fn cleanup(hwnd: HWND, handles: &Registrations) {
    if handles.source != 0 {
        // SAFETY: handle from a successful registration, released once.
        unsafe { UnregisterPowerSettingNotification(handles.source) };
    }
    if handles.session {
        // SAFETY: hwnd remains valid and registration succeeded.
        unsafe { WTSUnRegisterSessionNotification(hwnd) };
    }
    // SAFETY: watcher thread created and still owns hwnd.
    unsafe { DestroyWindow(hwnd) };
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_POWERBROADCAST => {
            let code = wparam as u32;
            match code {
                PBT_APMSUSPEND => events::emit("power:suspend", &json!({})),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                    events::emit("power:resume", &json!({}));
                },
                PBT_POWERSETTINGCHANGE => {
                    // SAFETY: for PBT_POWERSETTINGCHANGE, lparam points at a
                    // POWERBROADCAST_SETTING valid for the message.
                    if let Some(source) = unsafe { read_power_source(lparam) } {
                        report_power_source(source);
                    }
                },
                _ => {},
            }
            1
        },
        WM_ENDSESSION => {
            // wparam != 0: the session really ends (log off, restart, shut down).
            if wparam != 0 {
                events::emit("power:shutdown", &json!({}));
            }
            0
        },
        WM_WTSSESSION_CHANGE => {
            let code = wparam as u32;
            match code {
                WTS_SESSION_LOCK => events::emit("power:lock", &json!({})),
                WTS_SESSION_UNLOCK => events::emit("power:unlock", &json!({})),
                _ => {},
            }
            0
        },
        // SAFETY: standard forwarding for unhandled messages.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Read the DWORD carried by a `GUID_ACDC_POWER_SOURCE` notification.
///
/// # Safety
/// `lparam` must be the `POWERBROADCAST_SETTING` pointer of a
/// `PBT_POWERSETTINGCHANGE` message.
unsafe fn read_power_source(lparam: LPARAM) -> Option<u32> {
    let setting = lparam as *const POWERBROADCAST_SETTING;
    if setting.is_null() {
        return None;
    }
    // SAFETY: caller guarantees the pointer; read unaligned because the
    // struct ends in a variable-length byte array.
    let header = unsafe { std::ptr::read_unaligned(setting) };
    if !same_guid(&header.PowerSetting, &GUID_ACDC_POWER_SOURCE) || header.DataLength < 4 {
        return None;
    }
    // SAFETY: DataLength >= 4, so a DWORD fits in the trailing buffer.
    unsafe {
        let data = std::ptr::addr_of!((*setting).Data).cast::<[u8; 4]>();
        Some(u32::from_ne_bytes(std::ptr::read_unaligned(data)))
    }
}

/// `windows_sys::core::GUID` has no `PartialEq`.
fn same_guid(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

/// Emit `power:source-change` when the AC/battery state actually flips.
fn report_power_source(source: u32) {
    let current = u8::from(source != 0);
    let previous = LAST_SOURCE.swap(current, Ordering::AcqRel);
    if previous != SOURCE_UNKNOWN && previous != current {
        events::emit("power:source-change", &json!({ "onBattery": current == 1 }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_power_source_notification_only_seeds() {
        LAST_SOURCE.store(SOURCE_UNKNOWN, Ordering::Release);
        report_power_source(0);
        assert_eq!(LAST_SOURCE.load(Ordering::Acquire), 0);
        // UPS (2) counts as battery.
        report_power_source(2);
        assert_eq!(LAST_SOURCE.load(Ordering::Acquire), 1);
    }

    #[test]
    fn read_power_source_filters_by_guid() {
        let mut setting = POWERBROADCAST_SETTING {
            PowerSetting: GUID_ACDC_POWER_SOURCE,
            DataLength: 1,
            Data: [1],
        };
        // SAFETY: live struct; DataLength < 4 is rejected before reading Data.
        assert_eq!(
            unsafe { read_power_source(&raw mut setting as LPARAM) },
            None
        );
        setting.PowerSetting = GUID::from_u128(1);
        setting.DataLength = 4;
        // SAFETY: the GUID check fails before the trailing buffer is read.
        assert_eq!(
            unsafe { read_power_source(&raw mut setting as LPARAM) },
            None
        );
        // SAFETY: null is rejected up front.
        assert_eq!(unsafe { read_power_source(0) }, None);
    }
}
