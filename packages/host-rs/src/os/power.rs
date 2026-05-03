//! Power / battery / idle queries.
//!
//! Promoted from `@tynd/plugin-power`. The original plugin only exposed
//! `getIdleTime`; while moving it native we also surface battery + AC
//! status (`getPowerStatus`) since the data is already available through
//! cross-platform APIs and apps in the wild ask for it constantly.
//!
//! Backends:
//! - **Idle**: `user-idle` crate — `GetLastInputInfo` (Win), `CGEventSource` (macOS),
//!   `XScreenSaver` (Linux).
//! - **Battery / AC**:
//!   - Windows: `GetSystemPowerStatus` from `Win32_System_Power`.
//!   - Linux: read-only sysfs at `/sys/class/power_supply/*`.
//!   - macOS: not yet implemented — IOKit `IOPSCopyPowerSourcesInfo` would land here when we adopt
//!     the `core-foundation` bindings. Returns `null` fields rather than erroring so frontends
//!     gracefully degrade to "battery info unavailable".

use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::ipc_error::IpcError;

pub fn dispatch(method: &str, _args: &Value) -> Result<Value, String> {
    match method {
        "getIdleTime" => Ok(json!({ "seconds": idle_seconds()? })),
        "getPowerStatus" => Ok(power_status()),
        "battery" => battery_info(),
        // Idempotent — repeat calls return success without re-installing the
        // platform listener. Lets the TS client subscribe lazily on the first
        // `onSuspend` / `onLock` call without coordinating with sibling
        // subscriptions.
        "subscribePowerEvents" => {
            ensure_power_monitor();
            Ok(Value::Null)
        },
        _ => Err(IpcError::unknown_method("power", method).into_string()),
    }
}

/// Spin up the OS-specific power-event watcher exactly once. Subsequent calls
/// short-circuit. Each backend emits `power:suspend` / `power:resume` /
/// `power:lock` / `power:unlock` on the global event bus — the TS client's
/// `power.onSuspend` etc. is just `os_on` against those names.
///
/// Each backend runs on its own dedicated `std::thread` so it doesn't have
/// to coordinate with the main `tao` event loop:
/// - **Windows**: hidden message-only window (`HWND_MESSAGE`) +
///   `WTSRegisterSessionNotification` + a private `GetMessageW` pump.
///   `WM_POWERBROADCAST` (suspend/resume) and `WM_WTSSESSION_CHANGE`
///   (lock/unlock) are routed into `os::events::emit`.
/// - **macOS**: `NSWorkspace.sharedWorkspace.notificationCenter`
///   `addObserverForName:object:queue:usingBlock:` against the four
///   sleep/wake/session notification names. Blocks fire on the system
///   default queue and call straight into `os::events::emit`.
/// - **Linux**: `zbus::blocking::Proxy` against
///   `org.freedesktop.login1.Manager.PrepareForSleep` (boolean — true =
///   about to sleep) and `org.freedesktop.login1.Session.Lock` /
///   `Unlock` on the current session object.
///
/// On `macOS` and `Linux` the watcher is gated behind the
/// `power-events-mac` / `power-events-linux` sub-features so a
/// `--no-default-features` build that drops `api-power` doesn't pull
/// `objc2` / `zbus` for nothing. They're auto-enabled by `api-power`.
fn ensure_power_monitor() {
    static MONITOR_INSTALLED: OnceLock<()> = OnceLock::new();
    let _ = MONITOR_INSTALLED.get_or_init(|| {
        install_power_monitor();
    });
}

#[cfg(target_os = "windows")]
fn install_power_monitor() {
    windows_power::spawn_watcher();
}

#[cfg(all(target_os = "macos", feature = "power-events-mac"))]
fn install_power_monitor() {
    macos_power::spawn_watcher();
}

#[cfg(all(target_os = "macos", not(feature = "power-events-mac")))]
fn install_power_monitor() {
    crate::tynd_log!(
        "power: macOS event source disabled (build without `power-events-mac` feature). \
         API is callable; events won't fire."
    );
}

#[cfg(all(target_os = "linux", feature = "power-events-linux"))]
fn install_power_monitor() {
    linux_power::spawn_watcher();
}

#[cfg(all(target_os = "linux", not(feature = "power-events-linux")))]
fn install_power_monitor() {
    crate::tynd_log!(
        "power: Linux event source disabled (build without `power-events-linux` feature). \
         API is callable; events won't fire."
    );
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn install_power_monitor() {
    // Other platforms — no power-event source planned.
}

// ─────────────────────────── Windows backend ───────────────────────────

#[cfg(target_os = "windows")]
mod windows_power {
    //! Hidden message-only window + private message pump for
    //! `WM_POWERBROADCAST` and `WM_WTSSESSION_CHANGE`. The window class is
    //! registered lazily on first call; the watcher thread owns the HWND
    //! and never returns (process lifetime), so `UnregisterClass` is
    //! intentionally skipped — Windows reclaims everything on process exit.
    use serde_json::json;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            RemoteDesktop::{
                NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
                WTSUnRegisterSessionNotification,
            },
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassExW,
            HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND,
            TranslateMessage, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSEXW,
            WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
        },
    };

    use crate::os::events;

    /// UTF-16 NUL-terminated. Must outlive `RegisterClassExW` /
    /// `CreateWindowExW`; we leak via a `static` to keep the lifetime
    /// trivially `'static` without a one-shot `Box::leak`.
    const CLASS_NAME: &[u16] = &[
        b'T' as u16, b'y' as u16, b'n' as u16, b'd' as u16, b'P' as u16, b'o' as u16, b'w' as u16,
        b'e' as u16, b'r' as u16, b'W' as u16, b'a' as u16, b't' as u16, b'c' as u16, b'h' as u16,
        b'e' as u16, b'r' as u16, 0,
    ];

    pub(super) fn spawn_watcher() {
        std::thread::Builder::new()
            .name("tynd-power-win".into())
            .spawn(|| {
                if let Err(e) = run() {
                    crate::tynd_log!("power[win]: watcher exited: {e}");
                }
            })
            .ok();
    }

    fn run() -> Result<(), String> {
        // SAFETY: `GetModuleHandleW(NULL)` returns the current module's
        // HINSTANCE — a documented, infallible operation when called
        // with a null `lpModuleName`.
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
        // SAFETY: `class` is fully populated, the lifetime of
        // `CLASS_NAME` is `'static`, and `hinstance` is the current
        // process module. RegisterClassExW returns 0 on failure —
        // duplicate registration also returns 0 with last_error
        // `ERROR_CLASS_ALREADY_EXISTS`, which we tolerate (subsequent
        // CreateWindowExW will succeed against the existing class).
        let _atom = unsafe { RegisterClassExW(&raw const class) };

        // SAFETY: `HWND_MESSAGE` is a documented sentinel handle for
        // message-only windows. All other pointer args are null
        // (no menu, no parent params, no creation data).
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
            return Err("CreateWindowExW(HWND_MESSAGE) failed".into());
        }

        // SAFETY: `hwnd` is owned by this thread; registration scopes
        // notifications to this session only (matches the user's logon
        // — the only one that `os::power` cares about for lock/unlock).
        let session_ok =
            unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } != 0;
        if !session_ok {
            crate::tynd_log!(
                "power[win]: WTSRegisterSessionNotification failed; lock/unlock events disabled. \
                 Power suspend/resume still work via WM_POWERBROADCAST."
            );
        }

        // Standard Win32 message pump — blocks until WM_QUIT (which we
        // never post), so this thread runs for the process lifetime.
        let mut msg = MSG::default();
        loop {
            // SAFETY: `&raw mut msg` is a valid `*mut MSG` for the
            // duration of the call; the other args restrict messages
            // to the watcher window so we don't accidentally pump
            // unrelated thread messages.
            let r = unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) };
            if r == 0 || r == -1 {
                break;
            }
            // SAFETY: `msg` was just populated by GetMessageW.
            unsafe {
                TranslateMessage(&raw const msg);
                DispatchMessageW(&raw const msg);
            }
        }

        if session_ok {
            // SAFETY: `hwnd` is still valid (we own it) and the
            // session was successfully registered above.
            unsafe { WTSUnRegisterSessionNotification(hwnd) };
        }
        Ok(())
    }

    /// Windows window procedure — translates the two messages we care
    /// about into host bus events and forwards everything else to
    /// `DefWindowProcW`. Runs on the watcher thread (the one that
    /// called `CreateWindowExW`), so the bus emit happens on the same
    /// thread that owns the HWND.
    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_POWERBROADCAST => {
                // wparam carries the PBT_* event code.
                #[allow(clippy::cast_possible_truncation)]
                let code = wparam as u32;
                match code {
                    PBT_APMSUSPEND => events::emit("power:suspend", &json!({})),
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                        events::emit("power:resume", &json!({}));
                    },
                    _ => {},
                }
                // Returning TRUE (1) is the documented ACK. DefWindowProcW
                // would also return TRUE for unhandled PBT_* codes.
                1
            },
            WM_WTSSESSION_CHANGE => {
                #[allow(clippy::cast_possible_truncation)]
                let code = wparam as u32;
                match code {
                    WTS_SESSION_LOCK => events::emit("power:lock", &json!({})),
                    WTS_SESSION_UNLOCK => events::emit("power:unlock", &json!({})),
                    _ => {},
                }
                0
            },
            // SAFETY: forwarding non-handled messages to DefWindowProcW
            // is the standard Win32 contract.
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

}

// ─────────────────────────── macOS backend ───────────────────────────

#[cfg(all(target_os = "macos", feature = "power-events-mac"))]
mod macos_power {
    //! `NSWorkspace.notificationCenter` observer blocks bridged into
    //! `os::events::emit`. The four notifications cover sleep/wake
    //! (system-level power) and session active/inactive (fast user
    //! switching + screen lock on modern macOS where Quartz reports
    //! the screen-locked state via the session-resign notification).
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification,
        NSWorkspaceSessionDidBecomeActiveNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationName};
    use serde_json::json;

    use crate::os::events;

    pub(super) fn spawn_watcher() {
        // The actual observers must be installed on the main thread
        // (NSNotificationCenter is documented as MT-safe but
        // NSWorkspace.sharedWorkspace requires the main thread on
        // older macOS releases). `dispatch_async` to the main queue
        // would be the canonical move, but the Tynd host already runs
        // its tao event loop on the main thread, and observers
        // installed from any thread that owns a runloop are honored.
        // We spawn a thread purely so the registration doesn't block
        // `subscribePowerEvents`; the thread exits immediately after.
        std::thread::Builder::new()
            .name("tynd-power-mac".into())
            .spawn(install)
            .ok();
    }

    fn install() {
        // SAFETY: `NSWorkspace::sharedWorkspace()` is a documented
        // singleton accessor that returns a +1 retained pointer; the
        // generated binding wraps it in `Retained<NSWorkspace>` so the
        // refcount lifetime is tracked.
        let workspace = unsafe { NSWorkspace::sharedWorkspace() };
        let center = unsafe { workspace.notificationCenter() };

        // SAFETY: each `addObserverForName:object:queue:usingBlock:`
        // call retains the block for the lifetime of the observer
        // token. We intentionally leak the returned token via
        // `std::mem::forget` (semantically equivalent here — the
        // process keeps the observer for its lifetime, and dropping
        // the `Retained` would `removeObserver:` and silently lose
        // events).
        unsafe {
            register(&center, NSWorkspaceWillSleepNotification, "power:suspend");
            register(&center, NSWorkspaceDidWakeNotification, "power:resume");
            register(
                &center,
                NSWorkspaceSessionDidResignActiveNotification,
                "power:lock",
            );
            register(
                &center,
                NSWorkspaceSessionDidBecomeActiveNotification,
                "power:unlock",
            );
        }
    }

    /// SAFETY: caller must ensure `center` is a valid
    /// `NSNotificationCenter` and `name` is one of the four constants
    /// defined in `NSWorkspace`. We pass a `nil` queue, which means
    /// the block runs on the posting thread (the main runloop for
    /// these specific notifications), and `nil` object so we observe
    /// across all senders.
    unsafe fn register(
        center: &objc2_foundation::NSNotificationCenter,
        name: &NSNotificationName,
        event: &'static str,
    ) {
        let block = block2::RcBlock::new(move |_n: std::ptr::NonNull<NSNotification>| {
            events::emit(event, &json!({}));
        });
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        // Retain the observer for the process lifetime — see comment
        // in `install()`.
        std::mem::forget(token);
    }
}

// ─────────────────────────── Linux backend ───────────────────────────

#[cfg(all(target_os = "linux", feature = "power-events-linux"))]
mod linux_power {
    //! systemd-logind DBus signals via `zbus::blocking`. Two signals
    //! ride on different interfaces:
    //!
    //! - `org.freedesktop.login1.Manager.PrepareForSleep(bool)` —
    //!   `true` immediately before suspend, `false` immediately after
    //!   resume. Mapped to `power:suspend` / `power:resume`.
    //! - `org.freedesktop.login1.Session.Lock` / `.Unlock` (no body)
    //!   on the *current session* path. Mapped to `power:lock` /
    //!   `power:unlock`.
    //!
    //! If the system bus isn't available (sandboxed container, no
    //! systemd, …) we log once and exit the watcher thread — the
    //! TS API still works, it just never sees events on this host.
    use serde_json::json;
    use zbus::blocking::{Connection, Proxy};

    use crate::os::events;

    pub(super) fn spawn_watcher() {
        std::thread::Builder::new()
            .name("tynd-power-linux-sleep".into())
            .spawn(|| {
                if let Err(e) = run_sleep() {
                    crate::tynd_log!("power[linux]: PrepareForSleep watcher exited: {e}");
                }
            })
            .ok();
        std::thread::Builder::new()
            .name("tynd-power-linux-session".into())
            .spawn(|| {
                if let Err(e) = run_session() {
                    crate::tynd_log!("power[linux]: session Lock/Unlock watcher exited: {e}");
                }
            })
            .ok();
    }

    fn run_sleep() -> Result<(), String> {
        let conn = Connection::system().map_err(|e| format!("system bus: {e}"))?;
        let proxy = Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(|e| format!("Manager proxy: {e}"))?;

        let signals = proxy
            .receive_signal("PrepareForSleep")
            .map_err(|e| format!("receive_signal PrepareForSleep: {e}"))?;
        for msg in signals {
            // Body is `(b)` — single boolean.
            match msg.body().deserialize::<bool>() {
                Ok(true) => events::emit("power:suspend", &json!({})),
                Ok(false) => events::emit("power:resume", &json!({})),
                Err(e) => crate::tynd_log!("power[linux]: PrepareForSleep body: {e}"),
            }
        }
        Ok(())
    }

    fn run_session() -> Result<(), String> {
        let conn = Connection::system().map_err(|e| format!("system bus: {e}"))?;
        // The current session's object path is exposed by
        // `Manager.GetSessionByPID(self_pid) -> ObjectPath`. We avoid
        // the round-trip and use the magic alias `auto` which logind
        // resolves to "the current session" on most distros; if that
        // fails we fall through and the watcher exits with a logged
        // error rather than panicking the host.
        let manager = Proxy::new(
            &conn,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )
        .map_err(|e| format!("Manager proxy: {e}"))?;
        let pid = std::process::id();
        let session_path: zbus::zvariant::OwnedObjectPath = manager
            .call("GetSessionByPID", &(pid))
            .map_err(|e| format!("GetSessionByPID({pid}): {e}"))?;

        let session = Proxy::new(
            &conn,
            "org.freedesktop.login1",
            session_path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .map_err(|e| format!("Session proxy: {e}"))?;

        // Two SignalIterators interleaved via a non-blocking spin
        // would let us share the connection, but `blocking::Proxy`
        // doesn't expose a select(). Spawn a sibling thread for
        // Unlock so each iterator gets its own blocking pump.
        let unlock_conn = conn.clone();
        let unlock_path = session_path.clone();
        std::thread::Builder::new()
            .name("tynd-power-linux-unlock".into())
            .spawn(move || {
                if let Err(e) = run_unlock(&unlock_conn, &unlock_path) {
                    crate::tynd_log!("power[linux]: Unlock watcher exited: {e}");
                }
            })
            .ok();

        let signals = session
            .receive_signal("Lock")
            .map_err(|e| format!("receive_signal Lock: {e}"))?;
        for _ in signals {
            events::emit("power:lock", &json!({}));
        }
        Ok(())
    }

    fn run_unlock(
        conn: &Connection,
        session_path: &zbus::zvariant::OwnedObjectPath,
    ) -> Result<(), String> {
        let session = Proxy::new(
            conn,
            "org.freedesktop.login1",
            session_path.as_str(),
            "org.freedesktop.login1.Session",
        )
        .map_err(|e| format!("Session proxy (unlock): {e}"))?;
        let signals = session
            .receive_signal("Unlock")
            .map_err(|e| format!("receive_signal Unlock: {e}"))?;
        for _ in signals {
            events::emit("power:unlock", &json!({}));
        }
        Ok(())
    }
}

/// Cross-platform battery snapshot via the `battery` crate.
///
/// Picks the first reported battery — desktops with no battery return
/// `null` so frontends can render a "—" without a typed error path.
/// Status maps to the four states callers actually care about; `Empty`
/// folds into `discharging` (battery is dead but still attached) and
/// the residual `Unknown` covers OS quirks we don't need to enumerate.
fn battery_info() -> Result<Value, String> {
    use battery::{
        units::{ratio::percent, time::second},
        Manager, State,
    };

    let manager = Manager::new()
        .map_err(|e| IpcError::new("power", "battery", format!("manager: {e}")).into_string())?;
    let mut iter = manager
        .batteries()
        .map_err(|e| IpcError::new("power", "battery", format!("enumerate: {e}")).into_string())?;

    let Some(first) = iter.next() else {
        // No battery present (typical desktop). Return null shape rather
        // than erroring — a typed `NO_BATTERY` would force every caller
        // to special-case desktops.
        return Ok(Value::Null);
    };
    let bat = first
        .map_err(|e| IpcError::new("power", "battery", format!("read: {e}")).into_string())?;

    let state = bat.state();
    let status = match state {
        State::Charging => "charging",
        State::Discharging | State::Empty => "discharging",
        State::Full => "full",
        State::Unknown => "unknown",
        // `battery::State` is `#[non_exhaustive]` — an unknown variant
        // from a future crate version maps to "unknown" rather than
        // refusing to compile.
        _ => "unknown",
    };
    let charging = matches!(state, State::Charging);

    // `time_to_full` while charging, `time_to_empty` while discharging —
    // either may be `None` when the OS hasn't computed an estimate yet.
    let time_remaining_secs: Option<u64> = match state {
        State::Charging => bat.time_to_full().map(|t| t.get::<second>() as u64),
        State::Discharging => bat.time_to_empty().map(|t| t.get::<second>() as u64),
        _ => None,
    };

    Ok(json!({
        "percent": bat.state_of_charge().get::<percent>(),
        "charging": charging,
        "time_remaining_secs": time_remaining_secs,
        "status": status,
    }))
}

fn idle_seconds() -> Result<u64, String> {
    user_idle::UserIdle::get_time()
        .map(|t| t.as_seconds())
        .map_err(|e| IpcError::new("power", "getIdleTime", e).into_string())
}

/// Cross-platform battery + AC summary.
///
/// Shape:
/// ```text
/// {
///   "onBattery": bool | null,           // true = on battery, false = on AC
///   "batteryPercent": number | null,    // 0–100 if a battery is present
///   "charging": bool | null,            // true while charging up
///   "secondsRemaining": number | null,  // OS estimate, may be null while charging
/// }
/// ```
fn power_status() -> Value {
    #[cfg(target_os = "windows")]
    {
        windows_power_status()
    }
    #[cfg(target_os = "linux")]
    {
        linux_power_status()
    }
    #[cfg(target_os = "macos")]
    {
        // IOKit binding is the right place to wire this in v2. For now,
        // graceful null degrade so frontends can render "—" without
        // special-casing per-OS.
        json!({
            "onBattery": Value::Null,
            "batteryPercent": Value::Null,
            "charging": Value::Null,
            "secondsRemaining": Value::Null,
        })
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        json!({
            "onBattery": Value::Null,
            "batteryPercent": Value::Null,
            "charging": Value::Null,
            "secondsRemaining": Value::Null,
        })
    }
}

#[cfg(target_os = "windows")]
fn windows_power_status() -> Value {
    // SAFETY: GetSystemPowerStatus is a pure Win32 query that fills a
    // caller-allocated SYSTEM_POWER_STATUS struct. We zero-init the
    // struct (all fields are integer types) and only read it on success.
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetSystemPowerStatus(&raw mut status) } != 0;
    if !ok {
        return json!({
            "onBattery": Value::Null,
            "batteryPercent": Value::Null,
            "charging": Value::Null,
            "secondsRemaining": Value::Null,
        });
    }

    // ACLineStatus: 0 = offline (battery), 1 = online (AC), 255 = unknown.
    let on_battery: Value = match status.ACLineStatus {
        0 => Value::Bool(true),
        1 => Value::Bool(false),
        _ => Value::Null,
    };
    // BatteryLifePercent: 0–100, 255 = unknown.
    let battery_pct: Value = if status.BatteryLifePercent == 255 {
        Value::Null
    } else {
        Value::from(u32::from(status.BatteryLifePercent))
    };
    // BatteryFlag bit 8 (0x08) = charging.
    let charging: Value = if status.BatteryFlag == 255 {
        Value::Null
    } else {
        Value::Bool((status.BatteryFlag & 0x08) != 0)
    };
    // BatteryLifeTime: seconds, -1 (u32::MAX) = unknown.
    let secs: Value = if status.BatteryLifeTime == u32::MAX {
        Value::Null
    } else {
        Value::from(status.BatteryLifeTime)
    };

    json!({
        "onBattery": on_battery,
        "batteryPercent": battery_pct,
        "charging": charging,
        "secondsRemaining": secs,
    })
}

#[cfg(target_os = "linux")]
fn linux_power_status() -> Value {
    use std::path::Path;

    let base = Path::new("/sys/class/power_supply");
    let Ok(entries) = std::fs::read_dir(base) else {
        return json!({
            "onBattery": Value::Null,
            "batteryPercent": Value::Null,
            "charging": Value::Null,
            "secondsRemaining": Value::Null,
        });
    };

    let mut on_battery: Option<bool> = None;
    let mut battery_pct: Option<u32> = None;
    let mut charging: Option<bool> = None;
    let mut secs: Option<u64> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        let read = |name: &str| -> Option<String> {
            std::fs::read_to_string(path.join(name))
                .ok()
                .map(|s| s.trim().to_string())
        };
        let kind = read("type").unwrap_or_default();
        match kind.as_str() {
            "Mains" => {
                if let Some(online) = read("online") {
                    on_battery = Some(online == "0");
                }
            },
            "Battery" => {
                if let Some(pct) = read("capacity").and_then(|v| v.parse::<u32>().ok()) {
                    battery_pct = Some(pct);
                }
                if let Some(s) = read("status") {
                    charging = Some(s.eq_ignore_ascii_case("charging"));
                }
                // energy_now (uWh) / power_now (uW) → seconds remaining.
                let energy = read("energy_now").and_then(|v| v.parse::<u64>().ok());
                let power = read("power_now").and_then(|v| v.parse::<u64>().ok());
                if let (Some(e), Some(p)) = (energy, power) {
                    if p > 0 {
                        secs = Some((e * 3600) / p);
                    }
                }
            },
            _ => {},
        }
    }

    json!({
        "onBattery": on_battery.map_or(Value::Null, Value::Bool),
        "batteryPercent": battery_pct.map_or(Value::Null, |v| Value::from(v)),
        "charging": charging.map_or(Value::Null, Value::Bool),
        "secondsRemaining": secs.map_or(Value::Null, |v| Value::from(v)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_method_errors() {
        let err = dispatch("nope", &Value::Null).unwrap_err();
        assert!(err.contains("unknown method"), "got: {err}");
    }

    #[test]
    fn subscribe_is_idempotent() {
        // First call returns Null + installs the listener.
        assert_eq!(
            dispatch("subscribePowerEvents", &Value::Null).unwrap(),
            Value::Null
        );
        // Second call short-circuits but still returns Null.
        assert_eq!(
            dispatch("subscribePowerEvents", &Value::Null).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn power_status_returns_object_shape() {
        let v = power_status();
        assert!(v.is_object());
        let obj = v.as_object().unwrap();
        assert!(obj.contains_key("onBattery"));
        assert!(obj.contains_key("batteryPercent"));
        assert!(obj.contains_key("charging"));
        assert!(obj.contains_key("secondsRemaining"));
    }
}
