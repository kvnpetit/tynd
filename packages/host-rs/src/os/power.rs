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

use serde_json::{json, Value};

pub fn dispatch(method: &str, _args: &Value) -> Result<Value, String> {
    match method {
        "getIdleTime" => Ok(json!({ "seconds": idle_seconds()? })),
        "getPowerStatus" => Ok(power_status()),
        _ => Err(format!("power.{method}: unknown method")),
    }
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
