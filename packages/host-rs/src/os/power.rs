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
        _ => Err(format!("power.{method}: unknown method")),
    }
}

/// Spin up the OS-specific power-event watcher exactly once. Subsequent calls
/// short-circuit. Each backend emits `power:suspend` / `power:resume` /
/// `power:lock` / `power:unlock` on the global event bus — the TS client's
/// `power.onSuspend` etc. is just `os_on` against those names.
///
/// Platform status (current as of W7.3):
/// - **Windows**: deferred — `WM_POWERBROADCAST` and `WTS_SESSION_*` need a
///   message-only window hooked into the existing tao window proc, which is
///   a separate refactor. Subscribing succeeds; no events fire yet.
/// - **macOS**: deferred — `NSWorkspace` notification observers need an
///   Objective-C runtime bridge (objc2-app-kit) that's not currently wired.
/// - **Linux**: deferred — `org.freedesktop.login1` signals need a DBus
///   client (zbus) which isn't in the workspace deps yet.
///
/// The TS API works on every platform (subscribe + dispose are pure
/// frontend bookkeeping). When the platform backend lands the events flow
/// without any client-side change.
fn ensure_power_monitor() {
    static MONITOR_INSTALLED: OnceLock<()> = OnceLock::new();
    let _ = MONITOR_INSTALLED.get_or_init(|| {
        install_power_monitor();
    });
}

#[cfg(target_os = "windows")]
fn install_power_monitor() {
    crate::tynd_log!(
        "power: Windows event source not yet wired (WM_POWERBROADCAST + WTSRegisterSessionNotification \
         pending — tracking PR for hidden message-only window). API is callable; events won't fire \
         until the listener lands."
    );
}

#[cfg(target_os = "macos")]
fn install_power_monitor() {
    crate::tynd_log!(
        "power: macOS event source not yet wired (NSWorkspace observers pending objc2-app-kit \
         binding). API is callable; events won't fire until the listener lands."
    );
}

#[cfg(target_os = "linux")]
fn install_power_monitor() {
    crate::tynd_log!(
        "power: Linux event source not yet wired (org.freedesktop.login1 PrepareForSleep / \
         Lock signals pending zbus dep). API is callable; events won't fire until the listener \
         lands."
    );
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn install_power_monitor() {
    // Other platforms — no power-event source planned.
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

    let manager = Manager::new().map_err(|e| format!("power.battery: manager: {e}"))?;
    let mut iter = manager
        .batteries()
        .map_err(|e| format!("power.battery: enumerate: {e}"))?;

    let Some(first) = iter.next() else {
        // No battery present (typical desktop). Return null shape rather
        // than erroring — a typed `NO_BATTERY` would force every caller
        // to special-case desktops.
        return Ok(Value::Null);
    };
    let bat = first.map_err(|e| format!("power.battery: read: {e}"))?;

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
        .map_err(|e| format!("power.getIdleTime: {e}"))
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
