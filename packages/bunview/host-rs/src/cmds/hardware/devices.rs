// Auto-generated module split — edit freely.

use std::process::Command;
use serde_json::{json, Value};
use sysinfo::{Component, Components};

use super::{run_cmd, round1};

pub fn get_temperature(id: String) {
    super::spawn_response(id, temperature_impl);
}

pub(super) fn temperature_impl() -> Value {
    let components = Components::new_with_refreshed_list();

    // sysinfo returns components on Linux always, on macOS often, on Windows rarely
    // (requires specific WMI drivers). Fall back to a WMI query on Windows.
    if !components.is_empty() {
        return temperature_from_components(&components);
    }

    #[cfg(target_os = "windows")]
    if let Some(v) = windows_wmi_temperature() {
        return v;
    }

    json!({ "cpu": Value::Null, "components": Value::Array(vec![]) })
}

fn temperature_from_components(components: &Components) -> Value {
    let all: Vec<Value> = components
        .iter()
        .map(|c: &Component| json!({ "label": c.label(), "temp": round1(c.temperature() as f64) }))
        .collect();

    let cpu_temps: Vec<f64> = components
        .iter()
        .filter(|c: &&Component| {
            let l = c.label().to_lowercase();
            l.contains("core") || l.contains("cpu") || l.contains("tdie")
                || l.contains("tctl") || l.contains("package")
        })
        .map(|c: &Component| c.temperature() as f64)
        .collect();

    let cpu_avg = if cpu_temps.is_empty() {
        Value::Null
    } else {
        json!(round1(cpu_temps.iter().sum::<f64>() / cpu_temps.len() as f64))
    };

    json!({ "cpu": cpu_avg, "components": all })
}

#[cfg(target_os = "windows")]
fn windows_wmi_temperature() -> Option<Value> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-Command",
        "Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature \
         | Select-Object InstanceName,CurrentTemperature | ConvertTo-Json -Compress",
    ]);
    let out = run_cmd(cmd, 5000)?;
    let parsed: Value = serde_json::from_str(out.trim()).ok()?;

    let items: Vec<&Value> = if parsed.is_array() {
        parsed.as_array().unwrap().iter().collect()
    } else {
        vec![&parsed]
    };

    let components: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let label = item["InstanceName"].as_str()?.to_string();
            // CurrentTemperature is in tenths of Kelvin → °C
            let raw = item["CurrentTemperature"].as_f64()?;
            let celsius = round1(raw / 10.0 - 273.15);
            if !(0.0..=150.0).contains(&celsius) { return None; } // sanity check
            Some(json!({ "label": label, "temp": celsius }))
        })
        .collect();

    if components.is_empty() { return None; }

    let cpu_avg = {
        let vals: Vec<f64> = components.iter().filter_map(|c| c["temp"].as_f64()).collect();
        if vals.is_empty() { Value::Null }
        else { json!(round1(vals.iter().sum::<f64>() / vals.len() as f64)) }
    };

    Some(json!({ "cpu": cpu_avg, "components": components }))
}

pub fn get_usb_devices(id: String) {
    super::spawn_response(id, usb_devices_impl);
}

/// Cross-platform USB enumeration via `nusb` (pure-Rust, no libusb needed).
/// Uses native APIs: SetupAPI (Windows), IOKit (macOS), sysfs (Linux).
fn usb_devices_impl() -> Value {
    let Ok(devices) = nusb::list_devices() else { return json!([]); };

    let list: Vec<Value> = devices
        .map(|d| {
            let vid = d.vendor_id();
            let pid = d.product_id();
            let name = d.product_string()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("USB Device {:04X}:{:04X}", vid, pid));
            let manufacturer = d.manufacturer_string().unwrap_or("").to_string();
            json!({
                "name":         name,
                "manufacturer": manufacturer,
                "class":        usb_class_name(d.class()),
                "vendorId":     format!("{:04X}", vid),
                "productId":    format!("{:04X}", pid),
            })
        })
        .collect();

    json!(list)
}

/// USB device class code → human-readable name (USB spec §9.2.3).
fn usb_class_name(class: u8) -> &'static str {
    match class {
        0x00 => "Composite",
        0x01 => "Audio",
        0x02 => "CDC",
        0x03 => "HID",
        0x05 => "Physical",
        0x06 => "Image",
        0x07 => "Printer",
        0x08 => "Mass Storage",
        0x09 => "Hub",
        0x0A => "CDC Data",
        0x0B => "Smart Card",
        0x0D => "Content Security",
        0x0E => "Video",
        0x0F => "Healthcare",
        0x10 => "Audio/Video",
        0xDC => "Diagnostic",
        0xE0 => "Wireless",
        0xEF => "Miscellaneous",
        0xFE => "Application Specific",
        0xFF => "Vendor Specific",
        _    => "USB",
    }
}

pub fn get_audio_devices(id: String) {
    super::spawn_response(id, audio_devices_impl);
}

#[cfg(target_os = "windows")]
fn audio_devices_impl() -> Value {
    // Use MMDevice enumeration via PowerShell/AudioEndpoint for both inputs AND outputs.
    // Win32_SoundDevice only shows sound CARDS (outputs), not input devices.
    // AudioEndpoint class (from MMDeviceAPI) gives full input + output enumeration.
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-Command",
        // Render = output (DataFlow=0), Capture = input (DataFlow=1).
        // Fallback to Win32_SoundDevice if AudioEndpoint class not available.
        "try { \
            Get-CimInstance -Namespace root/cimv2 -ClassName Win32_PnPEntity | \
            Where-Object { $_.PNPClass -eq 'AudioEndpoint' -or $_.PNPClass -eq 'MEDIA' } | \
            Select-Object Name,Manufacturer,Status,PNPClass,PNPDeviceID | \
            ConvertTo-Json -Compress \
         } catch { \
            Get-CimInstance Win32_SoundDevice | \
            Select-Object Name,Manufacturer,Status | ConvertTo-Json -Compress \
         }",
    ]);
    let Some(out) = run_cmd(cmd, 6000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let items: Vec<&Value> = if parsed.is_array() {
        parsed.as_array().unwrap().iter().collect()
    } else {
        vec![&parsed]
    };

    let devices: Vec<Value> = items.iter().filter_map(|item| {
        let name = item["Name"].as_str()?.trim().to_string();
        if name.is_empty() { return None; }
        let manufacturer = item["Manufacturer"].as_str().unwrap_or("").trim().to_string();
        let status = item["Status"].as_str().unwrap_or("").trim().to_string();

        // Infer input vs output from device name when PNPClass isn't conclusive.
        // Microphone/webcam/input devices usually contain these keywords.
        let name_lower = name.to_lowercase();
        let kind = if name_lower.contains("microphone")
            || name_lower.contains("mic ")
            || name_lower.ends_with(" mic")
            || name_lower.contains("line in")
            || name_lower.contains("input")
            || name_lower.contains("capture")
            || name_lower.contains("webcam")
        {
            "input"
        } else {
            "output"
        };

        Some(json!({
            "name": name,
            "type": kind,
            "manufacturer": manufacturer,
            "status": status,
        }))
    }).collect();

    json!(devices)
}

#[cfg(target_os = "macos")]
fn audio_devices_impl() -> Value {
    let mut cmd = Command::new("system_profiler");
    cmd.args(["SPAudioDataType", "-json"]);
    let Some(out) = run_cmd(cmd, 6000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let entries = match parsed["SPAudioDataType"].as_array() {
        Some(a) => a,
        None => return json!([]),
    };

    let mut devices: Vec<Value> = Vec::new();
    for entry in entries {
        let name = entry["_name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() { continue; }
        // Determine type from sub-items
        let output = entry["coreaudio_default_audio_output_device"].as_str()
            .map(|s| s.eq_ignore_ascii_case("spaudio_yes"))
            .unwrap_or(false);
        let input = entry["coreaudio_default_audio_input_device"].as_str()
            .map(|s| s.eq_ignore_ascii_case("spaudio_yes"))
            .unwrap_or(false);
        let kind = if input { "input" } else if output { "output" } else { "output" };
        let manufacturer = entry["coreaudio_device_manufacturer"].as_str()
            .unwrap_or("").trim().to_string();
        devices.push(json!({
            "name": name,
            "type": kind,
            "manufacturer": manufacturer,
            "status": "OK",
        }));
    }
    json!(devices)
}

#[cfg(target_os = "linux")]
fn audio_devices_impl() -> Value {
    let mut devices: Vec<Value> = Vec::new();

    // Output devices (sinks)
    if let Some(out) = run_cmd({ let mut c = Command::new("pactl"); c.args(["list", "short", "sinks"]); c }, 3000) {
        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(5, '\t').collect();
            if parts.len() < 2 { continue; }
            let name = parts[1].trim().to_string();
            if name.is_empty() { continue; }
            devices.push(json!({ "name": name, "type": "output", "manufacturer": Value::Null, "status": "OK" }));
        }
    } else if let Some(out) = run_cmd({ let mut c = Command::new("aplay"); c.arg("-l"); c }, 3000) {
        // Fallback: aplay -l
        for line in out.lines() {
            if line.starts_with("card ") {
                let name = line.splitn(2, ':').nth(1).unwrap_or("").trim().to_string();
                if !name.is_empty() {
                    devices.push(json!({ "name": name, "type": "output", "manufacturer": Value::Null, "status": "OK" }));
                }
            }
        }
    }

    // Input devices (sources)
    if let Some(out) = run_cmd({ let mut c = Command::new("pactl"); c.args(["list", "short", "sources"]); c }, 3000) {
        for line in out.lines() {
            let parts: Vec<&str> = line.splitn(5, '\t').collect();
            if parts.len() < 2 { continue; }
            let name = parts[1].trim().to_string();
            // Skip monitor sources (loopback of outputs)
            if name.ends_with(".monitor") { continue; }
            if name.is_empty() { continue; }
            devices.push(json!({ "name": name, "type": "input", "manufacturer": Value::Null, "status": "OK" }));
        }
    }

    json!(devices)
}

pub fn get_display_info(id: String) {
    super::spawn_response(id, display_info_impl);
}

#[cfg(target_os = "windows")]
fn display_info_impl() -> Value {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-Command",
        "Get-CimInstance Win32_VideoController | \
         Select-Object Name,CurrentHorizontalResolution,CurrentVerticalResolution,CurrentRefreshRate | \
         ConvertTo-Json -Compress",
    ]);
    let Some(out) = run_cmd(cmd, 5000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let items: Vec<&Value> = if parsed.is_array() {
        parsed.as_array().unwrap().iter().collect()
    } else {
        vec![&parsed]
    };

    let displays: Vec<Value> = items.iter().filter_map(|item| {
        let name = item["Name"].as_str()?.trim().to_string();
        if name.is_empty() { return None; }
        let width  = item["CurrentHorizontalResolution"].as_u64();
        let height = item["CurrentVerticalResolution"].as_u64();
        let refresh = item["CurrentRefreshRate"].as_u64();
        // Only include entries that have an active resolution
        if width.is_none() && height.is_none() { return None; }
        Some(json!({
            "name": name,
            "width": width,
            "height": height,
            "refreshRate": refresh,
            "primary": Value::Null,
        }))
    }).collect();

    json!(displays)
}

#[cfg(target_os = "macos")]
fn display_info_impl() -> Value {
    let mut cmd = Command::new("system_profiler");
    cmd.args(["SPDisplaysDataType", "-json"]);
    let Some(out) = run_cmd(cmd, 6000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let entries = match parsed["SPDisplaysDataType"].as_array() {
        Some(a) => a,
        None => return json!([]),
    };

    let mut displays: Vec<Value> = Vec::new();
    for entry in entries {
        let name = entry["_name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() { continue; }

        // Parse resolution string: "1920 x 1080" or "2560 x 1600 Retina"
        let (width, height) = entry["spdisplays_resolution"]
            .as_str()
            .and_then(parse_resolution_str)
            .unwrap_or((None, None));

        // Parse refresh rate: "60 Hz" → 60
        let refresh: Option<u64> = entry["spdisplays_refresh_rate"]
            .as_str()
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok());

        let is_main = entry["spdisplays_main"].as_str()
            .map(|s| s.eq_ignore_ascii_case("spdisplays_yes"))
            .unwrap_or(false);

        displays.push(json!({
            "name": name,
            "width": width,
            "height": height,
            "refreshRate": refresh,
            "primary": is_main,
        }));
    }
    json!(displays)
}

#[cfg(target_os = "linux")]
fn display_info_impl() -> Value {
    // Try xrandr first
    if let Some(out) = run_cmd({ let mut c = Command::new("xrandr"); c.arg("--query"); c }, 3000) {
        return parse_xrandr(&out);
    }
    // Fallback: read connected DRM outputs from sysfs
    parse_drm_sysfs()
}

#[cfg(target_os = "linux")]
fn parse_xrandr(output: &str) -> Value {
    let mut displays: Vec<Value> = Vec::new();
    let mut primary_found = false;

    for line in output.lines() {
        if !line.contains(" connected") { continue; }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 { continue; }
        let name = parts[0].to_string();
        let is_primary = parts.contains(&"primary");
        if !primary_found && is_primary { primary_found = true; }

        // Find the resolution+offset pattern: "1920x1080+0+0"
        let res_part = parts.iter().find(|p| p.contains('x') && p.contains('+'));
        let (width, height) = res_part
            .and_then(|s| s.split('+').next())
            .and_then(parse_resolution_str)
            .unwrap_or((None, None));

        displays.push(json!({
            "name": name,
            "width": width,
            "height": height,
            "refreshRate": Value::Null,
            "primary": is_primary,
        }));
    }

    json!(displays)
}

#[cfg(target_os = "linux")]
fn parse_drm_sysfs() -> Value {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else { return json!([]); };
    let mut displays: Vec<Value> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let status_path = path.join("status");
        let modes_path  = path.join("modes");

        let Ok(status) = std::fs::read_to_string(&status_path) else { continue };
        if status.trim() != "connected" { continue; }

        let name = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        // First line of modes is the preferred/current resolution
        let (width, height) = std::fs::read_to_string(&modes_path)
            .ok()
            .as_deref()
            .and_then(|s| s.lines().next())
            .and_then(parse_resolution_str)
            .unwrap_or((None, None));

        displays.push(json!({
            "name": name,
            "width": width,
            "height": height,
            "refreshRate": Value::Null,
            "primary": Value::Null,
        }));
    }
    json!(displays)
}

/// Parse "1920 x 1080" or "2560 x 1600 Retina" → (width, height).
/// Used by display_info on macOS and Linux; unused on Windows.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn parse_resolution_str(s: &str) -> Option<(Option<u64>, Option<u64>)> {
    // Normalise "1920x1080" → split on 'x' or " x "
    let s = s.trim();
    let sep = if s.contains(" x ") { " x " } else { "x" };
    let mut parts = s.splitn(2, sep);
    let w: u64 = parts.next()?.trim().parse().ok()?;
    // Height may have trailing " Retina" etc — take the numeric prefix
    let h_str = parts.next()?.trim();
    let h: u64 = h_str.split_whitespace().next()?.parse().ok()?;
    Some((Some(w), Some(h)))
}
