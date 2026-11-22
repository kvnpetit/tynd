//! Hidden message-only window + private message pump for
//! `WM_POWERBROADCAST` and `WM_WTSSESSION_CHANGE`. The watcher thread
//! owns the HWND and stops on `WM_QUIT` during central host shutdown.
use std::sync::mpsc;

use serde_json::json;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        RemoteDesktop::{
            WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
            NOTIFY_FOR_THIS_SESSION,
        },
        Threading::GetCurrentThreadId,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        PeekMessageW, PostThreadMessageW, RegisterClassExW, TranslateMessage, HWND_MESSAGE, MSG,
        PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PM_NOREMOVE,
        WM_POWERBROADCAST, WM_QUIT, WM_WTSSESSION_CHANGE, WNDCLASSEXW, WTS_SESSION_LOCK,
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

    // SAFETY: HWND_MESSAGE creates a message-only, never-visible window.
    let hwnd: HWND = unsafe {
        CreateWindowExW(
            0,
            CLASS_NAME.as_ptr(),
            CLASS_NAME.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("power[win]: CreateWindowExW(HWND_MESSAGE) failed".into());
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

    if ready.send(Ok(())).is_err() {
        cleanup(hwnd, session_ok);
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

    cleanup(hwnd, session_ok);
    if pump_failed {
        Err("power[win]: GetMessageW failed".into())
    } else {
        Ok(())
    }
}

fn cleanup(hwnd: HWND, session_ok: bool) {
    if session_ok {
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
                _ => {},
            }
            1
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
