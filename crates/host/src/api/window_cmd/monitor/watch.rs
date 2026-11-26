//! OS sources of "the display configuration changed" for `monitor:change`.
//! Each one only calls [`super::notify_changed`]; the diff decides whether
//! anything the frontend sees actually changed. Installed once, on the main
//! thread, by the first `window.subscribeMonitorChanges`, and kept for the
//! life of the process.

use parking_lot::Mutex;

/// `true` once a source is running.
static INSTALLED: Mutex<bool> = Mutex::new(false);

pub(super) fn install() -> Result<(), String> {
    let mut installed = INSTALLED.lock();
    if !*installed {
        imp::install()?;
        *installed = true;
    }
    Ok(())
}

/// Windows: a hidden top-level window on its own thread (message-only
/// windows receive no broadcasts) catches `WM_DISPLAYCHANGE` (resolution,
/// monitor added / removed), `WM_DPICHANGED` and `WM_SETTINGCHANGE` with
/// `SPI_SETWORKAREA` (taskbar moved or resized). Bursts are coalesced with a
/// short timer before the event loop is asked to re-read the monitors.
#[cfg(target_os = "windows")]
mod imp {
    use std::sync::mpsc;

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer,
            RegisterClassExW, SetTimer, TranslateMessage, MSG, SPI_SETWORKAREA, WM_DISPLAYCHANGE,
            WM_DPICHANGED, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW,
        },
    };

    /// `"TyndDisplayWatcher"`, UTF-16 NUL-terminated.
    static CLASS_NAME: [u16; 19] = {
        let ascii = *b"TyndDisplayWatcher\0";
        let mut wide = [0u16; 19];
        let mut index = 0;
        while index < ascii.len() {
            wide[index] = ascii[index] as u16;
            index += 1;
        }
        wide
    };

    const DEBOUNCE_TIMER: usize = 1;
    const DEBOUNCE_MS: u32 = 250;

    pub(super) fn install() -> Result<(), String> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("tynd-display-watch".into())
            .spawn(move || run(&ready_tx))
            .map_err(|error| format!("display watcher thread spawn failed: {error}"))?;
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("display watcher did not start: {error}"))?
    }

    fn run(ready: &mpsc::SyncSender<Result<(), String>>) {
        // SAFETY: `GetModuleHandleW(NULL)` returns the current module.
        let hinstance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let class = WNDCLASSEXW {
            cbSize: u32::try_from(size_of::<WNDCLASSEXW>()).unwrap_or_default(),
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
        // SAFETY: the class is fully populated and CLASS_NAME is static.
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
            let _ = ready.send(Err("CreateWindowExW failed".into()));
            return;
        }
        if ready.send(Ok(())).is_err() {
            return;
        }
        let mut msg = MSG::default();
        // SAFETY: `msg` is writable storage owned by this thread; the pump
        // ends with the process (or on a GetMessageW error).
        while unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let relevant = msg == WM_DISPLAYCHANGE
            || msg == WM_DPICHANGED
            || (msg == WM_SETTINGCHANGE && wparam == SPI_SETWORKAREA as WPARAM);
        if relevant {
            // SAFETY: `hwnd` is this thread's window; re-arming an existing
            // timer id just restarts it.
            unsafe { SetTimer(hwnd, DEBOUNCE_TIMER, DEBOUNCE_MS, None) };
        } else if msg == WM_TIMER && wparam == DEBOUNCE_TIMER {
            // SAFETY: same window and timer id as `SetTimer` above.
            unsafe { KillTimer(hwnd, DEBOUNCE_TIMER) };
            super::super::notify_changed();
            return 0;
        }
        // SAFETY: forwarding the message unchanged to the default procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

/// macOS: `NSApplicationDidChangeScreenParametersNotification` (screens
/// added / removed / rearranged, resolution, scale, Dock and menu bar
/// changes), posted on the main thread.
#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::NSApplicationDidChangeScreenParametersNotification;
    use objc2_foundation::{NSNotification, NSNotificationCenter};

    pub(super) fn install() -> Result<(), String> {
        let block = block2::RcBlock::new(|_notification: std::ptr::NonNull<NSNotification>| {
            super::super::notify_changed();
        });
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: documented AppKit notification constant on the
        // process-wide center; nil object observes every sender, nil queue
        // runs the block on the posting (main) thread.
        // The notification center retains the observer for the process lifetime.
        let _token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidChangeScreenParametersNotification),
                None,
                None,
                &block,
            )
        };
        Ok(())
    }
}

/// Linux: GDK screen `monitors-changed` / `size-changed` plus each monitor's
/// `notify::workarea` and `notify::scale-factor`. Main thread only, like
/// every GDK call.
#[cfg(target_os = "linux")]
mod imp {
    use gtk::gdk::{self, prelude::MonitorExt};

    fn watch_monitor(monitor: &gdk::Monitor) {
        monitor.connect_workarea_notify(|_| super::super::notify_changed());
        monitor.connect_scale_factor_notify(|_| super::super::notify_changed());
    }

    pub(super) fn install() -> Result<(), String> {
        let screen = gdk::Screen::default().ok_or("no default GDK screen")?;
        screen.connect_monitors_changed(|_| super::super::notify_changed());
        screen.connect_size_changed(|_| super::super::notify_changed());
        let display = gdk::Display::default().ok_or("no default GDK display")?;
        for index in 0..display.n_monitors() {
            if let Some(monitor) = display.monitor(index) {
                watch_monitor(&monitor);
            }
        }
        display.connect_monitor_added(|_, monitor| {
            watch_monitor(monitor);
            super::super::notify_changed();
        });
        display.connect_monitor_removed(|_, _| super::super::notify_changed());
        Ok(())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod imp {
    pub(super) fn install() -> Result<(), String> {
        Err(format!(
            "monitor change events are not supported on {}",
            std::env::consts::OS
        ))
    }
}
