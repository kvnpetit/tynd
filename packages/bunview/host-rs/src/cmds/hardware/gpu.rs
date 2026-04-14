// Auto-generated module split — edit freely.

use std::process::Command;
use serde_json::{json, Value};

use super::run_cmd;

fn infer_gpu_type(name: &str, vram: Option<u64>) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("nvidia")
        || lower.contains("geforce")
        || lower.contains("quadro")
        || lower.contains("rtx")
        || lower.contains("gtx")
    {
        return "discrete";
    }
    if lower.contains("intel") {
        return "integrated";
    }
    if lower.contains("apple")
        || lower.contains("m1")
        || lower.contains("m2")
        || lower.contains("m3")
        || lower.contains("m4")
    {
        return "integrated";
    }
    if lower.contains("amd") || lower.contains("radeon") {
        return match vram {
            Some(v) if v > 1_000_000_000 => "discrete",
            Some(_) => "integrated",
            None => "unknown",
        };
    }
    "unknown"
}

pub fn get_gpu_usage(id: String) {
    super::spawn_response(id, gpu_usage_impl);
}

fn gpu_usage_impl() -> Value {
    // Try nvidia-smi first (all platforms)
    if let Some(nvidia) = try_nvidia_usage() {
        return nvidia;
    }

    // Platform fallbacks
    #[cfg(target_os = "windows")]
    {
        if let Some(wmi) = try_windows_wmi_gpu() {
            return wmi;
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(mac) = try_macos_gpu() {
            return mac;
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(amd) = try_rocm_gpu_usage() {
            return amd;
        }
    }

    json!([])
}

pub(super) fn try_nvidia_usage() -> Option<Value> {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw",
        "--format=csv,noheader,nounits",
    ]);
    let out = run_cmd(cmd, 3000)?;
    let mut gpus: Vec<Value> = Vec::new();

    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(6, ',').map(str::trim).collect();
        if parts.len() < 6 { continue; }
        let name = parts[0].to_string();
        let util: Option<f64> = parts[1].parse().ok();
        let vram_used: Option<u64> = parts[2].parse::<f64>().ok().map(|v| (v * 1_048_576.0) as u64);
        let vram_total: Option<u64> = parts[3].parse::<f64>().ok().map(|v| (v * 1_048_576.0) as u64);
        let temp: Option<f64> = parts[4].parse().ok();
        let power: Option<f64> = parts[5].parse().ok().filter(|&v: &f64| v > 0.0);

        gpus.push(json!({
            "name": name,
            "utilizationPercent": util,
            "vramUsed": vram_used,
            "vramTotal": vram_total,
            "temperatureCelsius": temp,
            "powerWatts": power,
        }));
    }

    if gpus.is_empty() { None } else { Some(json!(gpus)) }
}

#[cfg(target_os = "windows")]
fn try_windows_wmi_gpu() -> Option<Value> {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-CimInstance Win32_VideoController | \
         Select-Object Name,AdapterRAM | \
         ConvertTo-Json -Compress",
    ]);
    let out = run_cmd(cmd, 5000)?;
    let parsed: Value = serde_json::from_str(out.trim()).ok()?;

    let items: Vec<&Value> = if parsed.is_array() {
        parsed.as_array().unwrap().iter().collect()
    } else {
        vec![&parsed]
    };

    let gpus: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let name = item["Name"].as_str()?.trim().to_string();
            if name.is_empty() { return None; }
            let vram_total = item["AdapterRAM"].as_u64().filter(|&v| v > 0);
            Some(json!({
                "name": name,
                "utilizationPercent": Value::Null,
                "vramUsed": Value::Null,
                "vramTotal": vram_total,
                "temperatureCelsius": Value::Null,
                "powerWatts": Value::Null,
            }))
        })
        .collect();

    if gpus.is_empty() { None } else { Some(json!(gpus)) }
}

#[cfg(target_os = "macos")]
fn try_macos_gpu() -> Option<Value> {
    let mut cmd = Command::new("system_profiler");
    cmd.args(["SPDisplaysDataType", "-json"]);
    let out = run_cmd(cmd, 6000)?;
    let parsed: Value = serde_json::from_str(out.trim()).ok()?;

    let displays = parsed["SPDisplaysDataType"].as_array()?;
    let mut gpus: Vec<Value> = Vec::new();

    for display in displays {
        let name = display["spdisplays_vendor"]
            .as_str()
            .or_else(|| display["_name"].as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() { continue; }
        let vram_total = display["spdisplays_vram"]
            .as_str()
            .and_then(parse_vram_str);
        gpus.push(json!({
            "name": name,
            "utilizationPercent": Value::Null,
            "vramUsed": Value::Null,
            "vramTotal": vram_total,
            "temperatureCelsius": Value::Null,
            "powerWatts": Value::Null,
        }));
    }

    if gpus.is_empty() { None } else { Some(json!(gpus)) }
}

#[cfg(target_os = "linux")]
pub(super) fn try_rocm_gpu_usage() -> Option<Value> {
    let mut cmd = Command::new("rocm-smi");
    cmd.args(["--showuse", "--showtemp", "--showpower", "--showmeminfo", "vram", "--json"]);
    let out = run_cmd(cmd, 3000)?;
    let parsed: Value = serde_json::from_str(out.trim()).ok()?;

    let map = parsed.as_object()?;
    let mut gpus: Vec<Value> = Vec::new();

    for (key, val) in map {
        if !key.starts_with("card") { continue; }
        let name = val["Card Series"].as_str()
            .or_else(|| val["Card model"].as_str())
            .unwrap_or(key.as_str())
            .trim()
            .to_string();
        let util: Option<f64> = val["GPU use (%)"].as_str()
            .and_then(|s| s.parse().ok());
        let vram_used: Option<u64> = val["VRAM Total Used Memory (B)"].as_str()
            .and_then(|s| s.parse().ok());
        let vram_total: Option<u64> = val["VRAM Total Memory (B)"].as_str()
            .and_then(|s| s.parse().ok());
        let temp: Option<f64> = val["Temperature (Sensor edge) (C)"].as_str()
            .and_then(|s| s.parse().ok());
        let power: Option<f64> = val["Average Graphics Package Power (W)"].as_str()
            .and_then(|s| s.parse().ok());

        gpus.push(json!({
            "name": name,
            "utilizationPercent": util,
            "vramUsed": vram_used,
            "vramTotal": vram_total,
            "temperatureCelsius": temp,
            "powerWatts": power,
        }));
    }

    if gpus.is_empty() { None } else { Some(json!(gpus)) }
}

#[cfg(target_os = "windows")]
pub(super) fn get_gpu_list() -> Value {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Get-CimInstance Win32_VideoController | \
         Select-Object Name,AdapterRAM | \
         ConvertTo-Json -Compress",
    ]);
    let Some(out) = run_cmd(cmd, 5000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let items: Vec<&Value> = if parsed.is_array() {
        parsed.as_array().unwrap().iter().collect()
    } else {
        vec![&parsed]
    };

    let gpus: Vec<Value> = items
        .iter()
        .filter_map(|item| {
            let name = item["Name"].as_str()?.trim().to_string();
            if name.is_empty() { return None; }
            let vram = item["AdapterRAM"].as_u64().filter(|&v| v > 0);
            let gpu_type = infer_gpu_type(&name, vram);
            Some(json!({ "name": name, "vram": vram, "type": gpu_type }))
        })
        .collect();

    json!(gpus)
}

#[cfg(target_os = "macos")]
pub(super) fn get_gpu_list() -> Value {
    let mut cmd = Command::new("system_profiler");
    cmd.args(["SPDisplaysDataType", "-json"]);
    let Some(out) = run_cmd(cmd, 6000) else { return json!([]); };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else { return json!([]); };

    let mut gpus: Vec<Value> = vec![];
    if let Some(displays) = parsed["SPDisplaysDataType"].as_array() {
        for display in displays {
            if let Some(name) = display["spdisplays_vendor"]
                .as_str()
                .or_else(|| display["_name"].as_str())
            {
                let name = name.trim().to_string();
                if name.is_empty() { continue; }
                let vram = display["spdisplays_vram"]
                    .as_str()
                    .and_then(parse_vram_str);
                let gpu_type = infer_gpu_type(&name, vram);
                gpus.push(json!({ "name": name, "vram": vram, "type": gpu_type }));
            }
        }
    }
    json!(gpus)
}

#[cfg(target_os = "linux")]
pub(super) fn get_gpu_list() -> Value {
    // Pre-fetch NVIDIA VRAM via nvidia-smi (name → vram_bytes map)
    let nvidia_vram: std::collections::HashMap<String, u64> = {
        let mut cmd = Command::new("nvidia-smi");
        cmd.args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]);
        run_cmd(cmd, 3000)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(2, ',').map(str::trim);
                let name = parts.next()?.to_string();
                let vram = parts.next()?.parse::<f64>().ok()
                    .map(|v| (v * 1_048_576.0) as u64)?;
                Some((name, vram))
            })
            .collect()
    };

    let cmd = Command::new("lspci");
    let Some(out) = run_cmd(cmd, 4000) else { return json!([]); };
    let mut gpus: Vec<Value> = vec![];

    for line in out.lines() {
        let lower = line.to_lowercase();
        if lower.contains("vga")
            || lower.contains("display")
            || lower.contains("3d controller")
        {
            let name = line
                .splitn(2, ':')
                .nth(1)
                .and_then(|s| s.splitn(2, ':').nth(1))
                .unwrap_or(line)
                .trim()
                .to_string();

            // VRAM: nvidia-smi for NVIDIA, sysfs for AMD, null for others
            let vram = nvidia_vram.iter()
                .find(|(k, _)| name.to_lowercase().contains(&k.to_lowercase()))
                .map(|(_, &v)| v)
                .or_else(|| read_drm_vram(&name));

            let gpu_type = infer_gpu_type(&name, vram);
            gpus.push(json!({ "name": name, "vram": vram, "type": gpu_type }));
        }
    }
    json!(gpus)
}

#[cfg(target_os = "macos")]
fn parse_vram_str(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix(" GB").or_else(|| s.strip_suffix(" gb")) {
        return n.trim().parse::<f64>().ok().map(|v| (v * 1_000_000_000.0) as u64);
    }
    if let Some(n) = s.strip_suffix(" MB").or_else(|| s.strip_suffix(" mb")) {
        return n.trim().parse::<f64>().ok().map(|v| (v * 1_000_000.0) as u64);
    }
    None
}

#[cfg(target_os = "linux")]
fn read_drm_vram(gpu_name: &str) -> Option<u64> {
    let lower = gpu_name.to_lowercase();
    if !lower.contains("amd") && !lower.contains("radeon") {
        return None;
    }
    let entries = std::fs::read_dir("/sys/class/drm").ok()?;
    for entry in entries.flatten() {
        let path = entry.path().join("device/mem_info_vram_total");
        if let Ok(content) = std::fs::read_to_string(&path) {
            return content.trim().parse::<u64>().ok();
        }
    }
    None
}
