//! Power / battery / idle queries.
//!
//! Backends:
//! - **Idle**: `user-idle` crate — `GetLastInputInfo` (Win), `CGEventSource` (macOS),
//!   `XScreenSaver` (Linux).
//! - **Keep-awake** (`preventSleep`): see [`inhibit`] — power requests (Win), IOPM assertions
//!   (macOS), logind `Inhibit` (Linux).
//! - **Battery / AC**:
//!   - Windows: `GetSystemPowerStatus` from `Win32_System_Power`.
//!   - Linux: read-only sysfs at `/sys/class/power_supply/*`.
//!   - macOS: needs IOKit binding (`IOPSCopyPowerSourcesInfo`); until wired, every battery field
//!     returns null.

use std::{
    sync::LazyLock,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use parking_lot::{Condvar, Mutex};
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

mod inhibit;

const MONITOR_START_TIMEOUT: Duration = Duration::from_secs(2);
const MONITOR_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

fn join_workers(mut workers: Vec<JoinHandle<()>>, label: &str) {
    let current = std::thread::current().id();
    let deadline = Instant::now() + MONITOR_SHUTDOWN_TIMEOUT;

    while !workers.is_empty() {
        let mut index = workers.len();
        while index > 0 {
            index -= 1;
            if workers[index].thread().id() == current {
                crate::tynd_warn!("{label}: cannot join current thread; detaching it");
                workers.swap_remove(index);
            } else if workers[index].is_finished() {
                let worker = workers.swap_remove(index);
                if worker.join().is_err() {
                    crate::tynd_error!("{label}: worker panicked during shutdown");
                }
            }
        }

        if workers.is_empty() {
            return;
        }
        if Instant::now() >= deadline {
            crate::tynd_warn!(
                "{label}: shutdown deadline reached; detaching {} blocked worker(s)",
                workers.len()
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Dispatch marker — actual power state lives in the process-wide monitor
/// slot and the per-OS backend modules below.
#[derive(Debug, Default)]
pub(crate) struct PowerApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi {
    shutdown: Some(shutdown),
    release_webview: Some(release_webview),
    ..HostApi::new("power", |method, args| dispatch(method, &args))
};

#[tynd::handler(api = "power")]
impl PowerApi {
    /// Seconds since the user's last input event. Wrapped in
    /// `{ "seconds": u64 }` to leave room for future fields.
    pub fn get_idle_time() -> Result<Value, String> {
        Ok(json!({ "seconds": idle_seconds()? }))
    }

    /// Best-effort battery + AC summary. Fields default to `null` when the
    /// OS doesn't expose them.
    pub fn get_power_status() -> Value {
        power_status()
    }

    /// Detailed battery info — capacity / health / time-remaining. Errors
    /// when no battery is detected.
    pub fn battery() -> Result<Value, String> {
        battery_info()
    }

    /// Install the OS power-event watcher. Idempotent. After this returns,
    /// listeners on `power:suspend|resume|lock|unlock` start firing.
    pub fn subscribe_power_events() -> Result<(), String> {
        ensure_power_monitor()
    }

    /// Keep the system (and the display when `display` is true) awake until
    /// `allowSleep(id)` or host exit. Returns the request id.
    pub fn prevent_sleep(display: Option<bool>) -> Result<u64, String> {
        inhibit::prevent_sleep(display.unwrap_or(false))
    }

    /// Release a request made by `preventSleep`. Unknown or already
    /// released ids are ignored.
    pub fn allow_sleep(id: u64) {
        inhibit::allow_sleep(id);
    }
}

trait MonitorHandle {
    fn shutdown(self);
    fn is_alive(&self) -> bool;
}

enum MonitorState<T> {
    Vacant,
    Starting,
    Running(T),
    ShuttingDown,
}

struct MonitorLifecycle<T> {
    state: Mutex<MonitorState<T>>,
    changed: Condvar,
}

impl<T> Default for MonitorLifecycle<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(MonitorState::Vacant),
            changed: Condvar::new(),
        }
    }
}

impl<T: MonitorHandle> MonitorLifecycle<T> {
    fn ensure_with<F>(&self, install: F) -> Result<(), String>
    where
        F: FnOnce() -> Result<T, String>,
    {
        let mut install = Some(install);

        loop {
            let stale = {
                let mut state = self.state.lock();
                match &*state {
                    MonitorState::Running(monitor) if monitor.is_alive() => return Ok(()),
                    MonitorState::Running(_) => {
                        let MonitorState::Running(monitor) =
                            std::mem::replace(&mut *state, MonitorState::Starting)
                        else {
                            unreachable!();
                        };
                        Some(monitor)
                    },
                    MonitorState::Vacant => {
                        *state = MonitorState::Starting;
                        None
                    },
                    MonitorState::Starting => {
                        self.changed.wait(&mut state);
                        continue;
                    },
                    MonitorState::ShuttingDown => return Err(shutdown_error()),
                }
            };

            if let Some(stale) = stale {
                stale.shutdown();
            }

            let result = install.take().expect("installer consumed once")();
            let mut state = self.state.lock();
            if matches!(*state, MonitorState::ShuttingDown) {
                drop(state);
                if let Ok(monitor) = result {
                    monitor.shutdown();
                }
                self.changed.notify_all();
                return Err(shutdown_error());
            }

            match result {
                Ok(monitor) => {
                    *state = MonitorState::Running(monitor);
                    self.changed.notify_all();
                    return Ok(());
                },
                Err(error) => {
                    *state = MonitorState::Vacant;
                    self.changed.notify_all();
                    return Err(error);
                },
            }
        }
    }

    fn shutdown(&self) {
        let monitor = {
            let mut state = self.state.lock();
            match std::mem::replace(&mut *state, MonitorState::ShuttingDown) {
                MonitorState::Running(monitor) => Some(monitor),
                MonitorState::Vacant | MonitorState::Starting | MonitorState::ShuttingDown => None,
            }
        };
        self.changed.notify_all();
        if let Some(monitor) = monitor {
            monitor.shutdown();
        }
    }
}

fn shutdown_error() -> String {
    "power.subscribePowerEvents: host is shutting down".into()
}

#[cfg(target_os = "windows")]
type PlatformMonitor = windows_power::Monitor;
#[cfg(all(target_os = "macos", feature = "power-events-mac"))]
type PlatformMonitor = macos_power::Monitor;
#[cfg(all(target_os = "linux", feature = "power-events-linux"))]
type PlatformMonitor = linux_power::Monitor;
#[cfg(any(
    all(target_os = "macos", not(feature = "power-events-mac")),
    all(target_os = "linux", not(feature = "power-events-linux")),
    not(any(target_os = "windows", target_os = "macos", target_os = "linux"))
))]
type PlatformMonitor = NoopMonitor;

#[cfg(any(
    all(target_os = "macos", not(feature = "power-events-mac")),
    all(target_os = "linux", not(feature = "power-events-linux")),
    not(any(target_os = "windows", target_os = "macos", target_os = "linux"))
))]
struct NoopMonitor;

#[cfg(any(
    all(target_os = "macos", not(feature = "power-events-mac")),
    all(target_os = "linux", not(feature = "power-events-linux")),
    not(any(target_os = "windows", target_os = "macos", target_os = "linux"))
))]
impl MonitorHandle for NoopMonitor {
    fn shutdown(self) {}

    fn is_alive(&self) -> bool {
        false
    }
}

static MONITOR: LazyLock<MonitorLifecycle<PlatformMonitor>> =
    LazyLock::new(MonitorLifecycle::default);

/// Install the per-OS power-event watcher exactly once. Each backend emits
/// `power:{suspend,resume,lock,unlock}` via `crate::ipc::events::emit`.
///
/// Backends:
/// - Windows: hidden `HWND_MESSAGE` + `WTSRegisterSessionNotification` + `GetMessageW` pump.
///   `WM_POWERBROADCAST` and `WM_WTSSESSION_CHANGE`.
/// - macOS: `NSWorkspace` notification center observers (gated on `power-events-mac`).
/// - Linux: `zbus` proxy on `org.freedesktop.login1.*` (gated on `power-events-linux`).
fn ensure_power_monitor() -> Result<(), String> {
    MONITOR.ensure_with(install_power_monitor)
}

/// Release the keep-awake requests `label` made (its JS is gone).
pub(crate) fn release_webview(label: &str) {
    inhibit::release_owner(label);
}

pub(crate) fn shutdown() {
    inhibit::release_all();
    MONITOR.shutdown();
}

#[cfg(target_os = "windows")]
fn install_power_monitor() -> Result<PlatformMonitor, String> {
    windows_power::spawn_watcher()
}

#[cfg(all(target_os = "macos", feature = "power-events-mac"))]
fn install_power_monitor() -> Result<PlatformMonitor, String> {
    macos_power::spawn_watcher()
}

#[cfg(all(target_os = "macos", not(feature = "power-events-mac")))]
fn install_power_monitor() -> Result<PlatformMonitor, String> {
    Err("power.subscribePowerEvents: macOS event backend is disabled at compile time".into())
}

#[cfg(all(target_os = "linux", feature = "power-events-linux"))]
fn install_power_monitor() -> Result<PlatformMonitor, String> {
    linux_power::spawn_watcher()
}

#[cfg(all(target_os = "linux", not(feature = "power-events-linux")))]
fn install_power_monitor() -> Result<PlatformMonitor, String> {
    Err("power.subscribePowerEvents: Linux event backend is disabled at compile time".into())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn install_power_monitor() -> Result<PlatformMonitor, String> {
    Err("power.subscribePowerEvents: unsupported platform".into())
}

// ─────────────────────────── Windows backend ───────────────────────────

#[cfg(target_os = "windows")]
mod windows_power;

// ─────────────────────────── macOS backend ───────────────────────────

#[cfg(all(target_os = "macos", feature = "power-events-mac"))]
mod macos_power;

// ─────────────────────────── Linux backend ───────────────────────────

#[cfg(all(target_os = "linux", feature = "power-events-linux"))]
mod linux_power;

/// Cross-platform battery snapshot via the `starship-battery` crate.
/// First reported battery wins; desktops return `null`. `Empty` folds
/// into `discharging`.
fn battery_info() -> Result<Value, String> {
    use starship_battery::{
        units::{ratio::percent, time::second},
        Manager, State,
    };

    let manager = Manager::new()
        .map_err(|e| IpcError::new("power", "battery", format!("manager: {e}")).into_string())?;
    let mut iter = manager
        .batteries()
        .map_err(|e| IpcError::new("power", "battery", format!("enumerate: {e}")).into_string())?;

    let Some(first) = iter.next() else {
        // No battery (typical desktop) — null avoids forcing every caller
        // to special-case a typed NO_BATTERY error.
        return Ok(Value::Null);
    };
    let bat =
        first.map_err(|e| IpcError::new("power", "battery", format!("read: {e}")).into_string())?;

    let state = bat.state();
    let status = match state {
        State::Charging => "charging",
        State::Discharging | State::Empty => "discharging",
        // `Paused`: plugged in, charging held at the user-set charge limit.
        State::Full | State::Paused => "full",
        State::Unknown => "unknown",
    };
    let charging = matches!(state, State::Charging);

    // `time_to_full` while charging, `time_to_empty` while discharging —
    // either may be `None` when the OS hasn't computed an estimate yet.
    let seconds_remaining: Option<u64> = match state {
        State::Charging => bat.time_to_full().map(|t| t.get::<second>() as u64),
        State::Discharging => bat.time_to_empty().map(|t| t.get::<second>() as u64),
        _ => None,
    };

    Ok(json!({
        "percent": bat.state_of_charge().get::<percent>(),
        "charging": charging,
        "secondsRemaining": seconds_remaining,
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
        // macOS battery: needs IOKit binding; until wired, every field
        // returns null. Warn once per process so JS code branching on
        // `onBattery === true` won't silently miss low-power paths on
        // macOS laptops (mirrors `install_power_monitor`'s stub).
        static WARNED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        let () = *WARNED.get_or_init(|| {
            crate::tynd_warn!("power.status: macOS IOKit binding not wired — fields return null");
        });
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
        "batteryPercent": battery_pct.map_or(Value::Null, Value::from),
        "charging": charging.map_or(Value::Null, Value::Bool),
        "secondsRemaining": secs.map_or(Value::Null, Value::from),
    })
}
