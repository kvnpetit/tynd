// Auto-generated module split — edit freely.

use std::process::Command;
use serde_json::{json, Value};
use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, RefreshKind, System,
    Disk, DiskKind, Disks, ProcessRefreshKind, ProcessesToUpdate, Users,
};

use crate::ipc;
use super::{run_cmd, round1};
use super::gpu::get_gpu_list;

pub fn get_system_info(id: String) {
    std::thread::spawn(move || {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::new().with_cpu_usage().with_frequency())
                .with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();
        sys.refresh_memory();

        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        let physical_cores = sys.physical_core_count().unwrap_or(0);
        let logical_cores = sys.cpus().len();

        let result = json!({
            "cpu": {
                "name": cpu_name,
                "physicalCores": physical_cores,
                "logicalCores": logical_cores,
                "arch": std::env::consts::ARCH,
            },
            "memory": {
                "total": sys.total_memory(),
            },
            "gpus": get_gpu_list(),
            "os": {
                "name": System::name().unwrap_or_default(),
                "version": System::os_version().unwrap_or_default(),
                "kernel": System::kernel_version().unwrap_or_default(),
                "hostname": System::host_name().unwrap_or_default(),
            },
        });

        ipc::emit_response(&id, result);
    });
}

pub fn get_cpu_usage(id: String) {
    std::thread::spawn(move || {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::new().with_cpu_usage().with_frequency()),
        );
        sys.refresh_cpu_usage();
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();

        let cores: Vec<Value> = sys
            .cpus()
            .iter()
            .map(|c| {
                json!({
                    "usage": round1(c.cpu_usage() as f64),
                    "frequency": c.frequency(),
                })
            })
            .collect();

        ipc::emit_response(
            &id,
            json!({
                "global": round1(sys.global_cpu_usage() as f64),
                "cores": cores,
            }),
        );
    });
}

pub fn get_memory_info(id: String) {
    std::thread::spawn(move || {
        let mut sys = System::new_with_specifics(
            RefreshKind::new().with_memory(MemoryRefreshKind::everything()),
        );
        sys.refresh_memory();

        ipc::emit_response(
            &id,
            json!({
                "total":     sys.total_memory(),
                "used":      sys.used_memory(),
                "available": sys.available_memory(),
                "free":      sys.free_memory(),
                "totalSwap": sys.total_swap(),
                "usedSwap":  sys.used_swap(),
            }),
        );
    });
}

pub fn get_battery_info(id: String) {
    super::spawn_response(id, battery_info_impl);
}

/// Cross-platform battery info via `starship-battery` crate.
/// Handles Windows (WMI), macOS (IOKit), and Linux (sysfs) uniformly.
fn battery_info_impl() -> Value {
    use starship_battery::{Manager, State};
    use starship_battery::units::ratio::percent;
    use starship_battery::units::time::minute;

    let Ok(manager) = Manager::new() else {
        return json!({ "present": false });
    };
    let Ok(mut batteries) = manager.batteries() else {
        return json!({ "present": false });
    };
    let Some(Ok(battery)) = batteries.next() else {
        return json!({ "present": false });
    };

    let level = battery.state_of_charge().get::<percent>() as f64;
    let state = battery.state();
    let charging   = state == State::Charging;
    let plugged_in = matches!(state, State::Charging | State::Full);

    // Health = energy_full / energy_full_design × 100
    let full   = battery.energy_full().value as f64;
    let design = battery.energy_full_design().value as f64;
    let health = if design > 0.0 {
        Some(round1(full / design * 100.0))
    } else {
        None
    };

    // Time remaining: to full when charging, to empty when discharging
    let time_remaining = if charging {
        battery.time_to_full().map(|t| round1(t.get::<minute>() as f64))
    } else {
        battery.time_to_empty().map(|t| round1(t.get::<minute>() as f64))
    };

    json!({
        "present": true,
        "level": round1(level),
        "charging": charging,
        "pluggedIn": plugged_in,
        "health": health,
        "timeRemaining": time_remaining,
    })
}

pub fn get_disk_info(id: String) {
    std::thread::spawn(move || {
        let disks = Disks::new_with_refreshed_list();

        let list: Vec<Value> = disks
            .iter()
            .map(|d: &Disk| {
                let name = d.name().to_string_lossy().to_string();
                let label = d.mount_point().to_string_lossy().to_string();
                let total = d.total_space();
                let available = d.available_space();
                let used = total.saturating_sub(available);
                let fs = d.file_system().to_string_lossy().to_string();
                let removable = d.is_removable();
                let kind = match d.kind() {
                    DiskKind::SSD => "SSD",
                    DiskKind::HDD => "HDD",
                    _ => "Unknown",
                };
                json!({
                    "name": name,
                    "label": label,
                    "totalSpace": total,
                    "availableSpace": available,
                    "usedSpace": used,
                    "filesystem": fs,
                    "removable": removable,
                    "type": kind,
                })
            })
            .collect();

        ipc::emit_response(&id, json!(list));
    });
}

pub fn get_process_list(id: String) {
    std::thread::spawn(move || {
        let mut sys = System::new();
        // Two refreshes for accurate CPU usage
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );

        let mut processes: Vec<Value> = sys.processes().iter()
            .map(|(pid, p)| {
                json!({
                    "pid":         usize::from(*pid) as u32,
                    "name":        p.name().to_string_lossy().as_ref(),
                    "cpuUsage":    round1(p.cpu_usage() as f64),
                    "memoryBytes": p.memory(),
                    "status":      format!("{:?}", p.status()),
                })
            })
            .collect();

        // Sort by CPU usage descending, take top 50
        processes.sort_by(|a, b| {
            b["cpuUsage"].as_f64().unwrap_or(0.0)
                .partial_cmp(&a["cpuUsage"].as_f64().unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        processes.truncate(50);

        ipc::emit_response(&id, json!(processes));
    });
}

pub fn get_users(id: String) {
    std::thread::spawn(move || {
        let users = Users::new_with_refreshed_list();
        let list: Vec<Value> = users.iter().map(|u| {
            json!({ "name": u.name() })
        }).collect();
        ipc::emit_response(&id, json!(list));
    });
}

pub fn get_cpu_details(id: String) {
    super::spawn_response(id, cpu_details_impl);
}

#[cfg(target_os = "windows")]
fn cpu_details_impl() -> Value {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-Command",
        "Get-CimInstance Win32_Processor | \
         Select-Object Name,Manufacturer,L2CacheSize,L3CacheSize,MaxClockSpeed,\
         NumberOfCores,NumberOfLogicalProcessors | \
         ConvertTo-Json -Compress",
    ]);
    let Some(out) = run_cmd(cmd, 5000) else { return json!({}) };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!({}) };

    let item = if parsed.is_array() {
        parsed.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null)
    } else {
        parsed
    };

    json!({
        "name":         item["Name"].as_str().unwrap_or("").trim(),
        "vendor":       item["Manufacturer"].as_str().unwrap_or("").trim(),
        "l2CacheKb":    item["L2CacheSize"].as_u64(),
        "l3CacheKb":    item["L3CacheSize"].as_u64(),
        "maxClockMhz":  item["MaxClockSpeed"].as_u64(),
        "physicalCores":item["NumberOfCores"].as_u64(),
        "logicalCores": item["NumberOfLogicalProcessors"].as_u64(),
    })
}

#[cfg(target_os = "macos")]
fn cpu_details_impl() -> Value {
    let mut cmd = Command::new("system_profiler");
    cmd.args(["SPHardwareDataType", "-json"]);
    let Some(out) = run_cmd(cmd, 6000) else { return json!({}) };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!({}) };

    let hw = match parsed["SPHardwareDataType"].as_array().and_then(|a| a.first()) {
        Some(v) => v.clone(),
        None => return json!({}),
    };

    // Chip name from "chip_type" (e.g. "Apple M2 Pro") or "cpu_type"
    let name = hw["chip_type"]
        .as_str()
        .or_else(|| hw["cpu_type"].as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    // Parse "10 GHz" → 10000 MHz from "current_processor_speed"
    let max_clock_mhz: Option<u64> = hw["current_processor_speed"]
        .as_str()
        .and_then(parse_clock_speed_mhz);

    // macOS `number_processors` has two formats:
    //   - Intel Mac: plain integer like "8"
    //   - Apple Silicon: "proc 12:8:4" meaning 12 total : 8 P-cores : 4 E-cores
    //     (Apple Silicon has no SMT, so physical == logical)
    let (physical_cores, logical_cores) = hw["number_processors"].as_str()
        .map(parse_mac_processor_count)
        .unwrap_or((None, None));

    // Fallback for Intel Macs if number_processors wasn't parseable
    let physical_cores = physical_cores.or_else(|| hw["physical_processor_count"].as_u64());
    let logical_cores  = logical_cores.or_else(|| hw["logical_processors"].as_u64());

    json!({
        "name":         name,
        "vendor":       "Apple",
        "l2CacheKb":    Value::Null,
        "l3CacheKb":    Value::Null,
        "maxClockMhz":  max_clock_mhz,
        "physicalCores":physical_cores,
        "logicalCores": logical_cores,
    })
}

#[cfg(target_os = "linux")]
fn cpu_details_impl() -> Value {
    let Ok(content) = std::fs::read_to_string("/proc/cpuinfo") else { return json!({}) };

    let mut name = String::new();
    let mut vendor = String::new();
    let mut cache_kb: Option<u64> = None;
    let mut max_mhz: Option<f64> = None;
    let mut physical_cores: Option<u64> = None;
    let mut core_ids = std::collections::HashSet::new();

    for line in content.lines() {
        if let Some(val) = line.strip_prefix("model name\t: ") {
            if name.is_empty() { name = val.trim().to_string(); }
        } else if let Some(val) = line.strip_prefix("vendor_id\t: ") {
            if vendor.is_empty() { vendor = val.trim().to_string(); }
        } else if let Some(val) = line.strip_prefix("cache size\t: ") {
            // "6144 KB"
            if cache_kb.is_none() {
                cache_kb = val.split_whitespace().next().and_then(|s| s.parse().ok());
            }
        } else if let Some(val) = line.strip_prefix("cpu MHz\t\t: ")
            .or_else(|| line.strip_prefix("cpu MHz\t: ")) {
            if let Ok(mhz) = val.trim().parse::<f64>() {
                max_mhz = Some(max_mhz.map(|m| m.max(mhz)).unwrap_or(mhz));
            }
        } else if let Some(val) = line.strip_prefix("core id\t\t: ")
            .or_else(|| line.strip_prefix("core id\t: ")) {
            core_ids.insert(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("cpu cores\t: ") {
            if physical_cores.is_none() {
                physical_cores = val.trim().parse().ok();
            }
        }
    }

    if physical_cores.is_none() && !core_ids.is_empty() {
        physical_cores = Some(core_ids.len() as u64);
    }

    // Try to get max frequency from cpufreq
    let max_from_sys: Option<u64> = std::fs::read_to_string(
        "/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq"
    ).ok().as_deref().and_then(|s| s.trim().parse::<u64>().ok()).map(|khz| khz / 1000);

    let max_clock_mhz = max_from_sys
        .or_else(|| max_mhz.map(|m| m as u64));

    json!({
        "name":         name,
        "vendor":       vendor,
        "l2CacheKb":    Value::Null,
        "l3CacheKb":    cache_kb,
        "maxClockMhz":  max_clock_mhz,
        "physicalCores":physical_cores,
        "logicalCores": Value::Null,
    })
}

pub fn get_ram_details(id: String) {
    super::spawn_response(id, ram_details_impl);
}

#[cfg(target_os = "windows")]
fn ram_details_impl() -> Value {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-Command",
        "Get-CimInstance Win32_PhysicalMemory | \
         Select-Object Manufacturer,PartNumber,Capacity,Speed,ConfiguredClockSpeed,SMBIOSMemoryType | \
         ConvertTo-Json -Compress",
    ]);
    let Some(out) = run_cmd(cmd, 5000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let items: Vec<&Value> = if parsed.is_array() {
        parsed.as_array().unwrap().iter().collect()
    } else {
        vec![&parsed]
    };

    let modules: Vec<Value> = items.iter().filter_map(|item| {
        let capacity = item["Capacity"].as_u64().filter(|&v| v > 0)?;
        let manufacturer = item["Manufacturer"].as_str()
            .map(str::trim)
            .unwrap_or("Unknown")
            .to_string();
        let part_number = item["PartNumber"].as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // ConfiguredClockSpeed is the actual running speed; Speed is the rated max
        let speed_mhz = item["ConfiguredClockSpeed"].as_u64()
            .filter(|&v| v > 0)
            .or_else(|| item["Speed"].as_u64().filter(|&v| v > 0));
        // SMBIOSMemoryType: 26=DDR4, 34=DDR5, 24=DDR3, 29=LPDDR3, 43=LPDDR4, 44=LPDDR5
        let ram_type = match item["SMBIOSMemoryType"].as_u64().unwrap_or(0) {
            20 => "DDR",
            21 => "DDR2",
            24 => "DDR3",
            26 => "DDR4",
            29 => "LPDDR3",
            43 => "LPDDR4",
            44 => "LPDDR5",
            34 => "DDR5",
            _  => "Unknown",
        };
        Some(json!({
            "capacityBytes": capacity,
            "type": ram_type,
            "speedMhz": speed_mhz,
            "manufacturer": manufacturer,
            "partNumber": part_number,
        }))
    }).collect();

    json!(modules)
}

#[cfg(target_os = "macos")]
fn ram_details_impl() -> Value {
    let mut cmd = Command::new("system_profiler");
    cmd.args(["SPMemoryDataType", "-json"]);
    let Some(out) = run_cmd(cmd, 6000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let entries = match parsed["SPMemoryDataType"].as_array() {
        Some(a) => a,
        None => return json!([]),
    };

    let mut modules: Vec<Value> = Vec::new();
    for entry in entries {
        // Each entry may have sub-items (individual DIMMs) or be a unified module
        if let Some(items) = entry["_items"].as_array() {
            for item in items {
                if let Some(m) = parse_macos_dimm(item) { modules.push(m); }
            }
        } else if let Some(m) = parse_macos_dimm(entry) {
            modules.push(m);
        }
    }
    json!(modules)
}

#[cfg(target_os = "macos")]
fn parse_macos_dimm(item: &Value) -> Option<Value> {
    // "dimm_size": "8 GB", "dimm_type": "LPDDR5", "dimm_speed": "6400 MHz",
    // "dimm_manufacturer": "Apple", "dimm_part": "..."
    let size_str = item["dimm_size"].as_str()?;
    let capacity_bytes = parse_size_to_bytes(size_str)?;
    let ram_type = item["dimm_type"].as_str().unwrap_or("Unknown").trim().to_string();
    let speed_mhz: Option<u64> = item["dimm_speed"].as_str()
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok());
    let manufacturer = item["dimm_manufacturer"].as_str()
        .unwrap_or("Unknown").trim().to_string();
    let part_number = item["dimm_part"].as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Some(json!({
        "capacityBytes": capacity_bytes,
        "type": ram_type,
        "speedMhz": speed_mhz,
        "manufacturer": manufacturer,
        "partNumber": part_number,
    }))
}

#[cfg(target_os = "macos")]
fn parse_size_to_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix(" GB") {
        return n.trim().parse::<f64>().ok().map(|v| (v * 1_073_741_824.0) as u64);
    }
    if let Some(n) = s.strip_suffix(" MB") {
        return n.trim().parse::<f64>().ok().map(|v| (v * 1_048_576.0) as u64);
    }
    None
}

#[cfg(target_os = "linux")]
fn ram_details_impl() -> Value {
    // dmidecode --type 17 gives memory device details (may need root on some distros)
    let mut cmd = Command::new("dmidecode");
    cmd.args(["--type", "17"]);
    let Some(out) = run_cmd(cmd, 4000) else { return json!([]); };

    let mut modules: Vec<Value> = Vec::new();
    let mut current: std::collections::HashMap<&str, String> = std::collections::HashMap::new();

    for line in out.lines() {
        let line = line.trim();
        if line.starts_with("Memory Device") {
            if !current.is_empty() {
                if let Some(m) = build_linux_dimm(&current) { modules.push(m); }
                current.clear();
            }
        } else if let Some((key, val)) = line.split_once(':') {
            current.insert(key.trim(), val.trim().to_string());
        }
    }
    // Last block
    if let Some(m) = build_linux_dimm(&current) { modules.push(m); }

    json!(modules)
}

#[cfg(target_os = "linux")]
fn build_linux_dimm(fields: &std::collections::HashMap<&str, String>) -> Option<Value> {
    let size_str = fields.get("Size")?;
    if size_str == "No Module Installed" || size_str == "Not Installed" { return None; }
    // "8192 MB" or "16 GB"
    let capacity_bytes: Option<u64> = parse_linux_size(size_str);
    let ram_type = fields.get("Type").map(String::as_str).unwrap_or("Unknown").to_string();
    let speed_mhz: Option<u64> = fields.get("Configured Memory Speed")
        .or_else(|| fields.get("Speed"))
        .and_then(|s| s.split_whitespace().next())
        .and_then(|s| s.parse().ok());
    let manufacturer = fields.get("Manufacturer")
        .map(String::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let part_number = fields.get("Part Number")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "Not Specified");
    Some(json!({
        "capacityBytes": capacity_bytes,
        "type": ram_type,
        "speedMhz": speed_mhz,
        "manufacturer": manufacturer,
        "partNumber": part_number,
    }))
}

#[cfg(target_os = "linux")]
fn parse_linux_size(s: &str) -> Option<u64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 { return None; }
    let n: f64 = parts[0].parse().ok()?;
    match parts[1] {
        "GB" => Some((n * 1_073_741_824.0) as u64),
        "MB" => Some((n * 1_048_576.0) as u64),
        "KB" => Some((n * 1_024.0) as u64),
        _ => None,
    }
}


/// macOS `number_processors` parser.
/// - Apple Silicon: "proc 12:8:4" → (Some(12 physical), Some(12 logical)) [no SMT]
/// - Intel: "8" → (None, Some(8))  [logical only; physical unknown w/o HT info]
#[cfg(target_os = "macos")]
fn parse_mac_processor_count(s: &str) -> (Option<u64>, Option<u64>) {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("proc ") {
        // Apple Silicon format
        if let Some(total_str) = rest.split(':').next() {
            if let Ok(total) = total_str.trim().parse::<u64>() {
                return (Some(total), Some(total));
            }
        }
    }
    // Intel format: plain integer
    if let Ok(n) = s.parse::<u64>() {
        return (None, Some(n));
    }
    (None, None)
}

/// Parse "3.49 GHz" or "2400 MHz" → MHz as u64
#[cfg(target_os = "macos")]
fn parse_clock_speed_mhz(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(ghz) = s.strip_suffix(" GHz") {
        return ghz.trim().parse::<f64>().ok().map(|g| (g * 1000.0) as u64);
    }
    if let Some(mhz) = s.strip_suffix(" MHz") {
        return mhz.trim().parse::<u64>().ok();
    }
    None
}
