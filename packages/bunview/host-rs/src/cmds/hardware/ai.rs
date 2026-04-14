// Auto-generated module split — edit freely.

use std::process::Command;
use serde_json::{json, Value};

use super::run_cmd;

pub fn get_ai_capabilities(id: String) {
    super::spawn_response(id, ai_capabilities_impl);
}

fn ai_capabilities_impl() -> Value {
    let cuda   = detect_cuda();
    let rocm   = detect_rocm();
    let vulkan = detect_vulkan();
    let npu    = detect_npu();
    let directml = detect_directml();
    let webnn    = detect_webnn();

    #[cfg(target_os = "macos")]
    let metal = json!({ "available": true });
    #[cfg(not(target_os = "macos"))]
    let metal = json!({ "available": false });

    json!({
        "cuda":     cuda,
        "rocm":     rocm,
        "vulkan":   vulkan,
        "metal":    metal,
        "directml": directml,
        "webnn":    webnn,
        "npu":      npu,
    })
}

fn detect_cuda() -> Value {
    // Query GPU compute capability + VRAM
    let mut cmd = Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=name,compute_cap,memory.total",
        "--format=csv,noheader,nounits",
    ]);
    let Some(out) = run_cmd(cmd, 3000) else {
        return json!({ "available": false, "version": Value::Null, "devices": [] });
    };

    let mut devices: Vec<Value> = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(3, ',').map(str::trim).collect();
        if parts.len() < 3 { continue; }
        let name = parts[0].to_string();
        let compute_cap = parts[1].to_string();
        let vram: Option<u64> = parts[2].parse::<f64>().ok().map(|v| (v * 1_048_576.0) as u64);
        devices.push(json!({
            "name": name,
            "computeCapability": compute_cap,
            "vram": vram,
        }));
    }

    if devices.is_empty() {
        return json!({ "available": false, "version": Value::Null, "devices": [] });
    }

    // Get CUDA version from nvidia-smi --version or xml query
    let cuda_version = detect_cuda_version();

    json!({
        "available": true,
        "version": cuda_version,
        "devices": devices,
    })
}

fn detect_cuda_version() -> Value {
    // nvidia-smi output first line usually contains "CUDA Version: X.Y"
    let cmd = Command::new("nvidia-smi");
    let Some(out) = run_cmd(cmd, 3000) else { return Value::Null; };
    for line in out.lines() {
        if let Some(rest) = line.find("CUDA Version:").map(|i| &line[i + 13..]) {
            let ver = rest.trim().split_whitespace().next().unwrap_or("").to_string();
            if !ver.is_empty() {
                return json!(ver);
            }
        }
    }
    Value::Null
}

fn detect_rocm() -> Value {
    let mut cmd = Command::new("rocm-smi");
    cmd.args(["--showproductname", "--json"]);
    let Some(out) = run_cmd(cmd, 3000) else {
        return json!({ "available": false, "version": Value::Null, "devices": [] });
    };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else {
        return json!({ "available": false, "version": Value::Null, "devices": [] });
    };

    let map = match parsed.as_object() {
        Some(m) => m,
        None => return json!({ "available": false, "version": Value::Null, "devices": [] }),
    };

    let mut devices: Vec<Value> = Vec::new();
    for (key, val) in map {
        if !key.starts_with("card") { continue; }
        let name = val["Card Series"].as_str()
            .or_else(|| val["Card model"].as_str())
            .unwrap_or(key.as_str())
            .trim()
            .to_string();
        devices.push(json!({ "name": name }));
    }

    if devices.is_empty() {
        return json!({ "available": false, "version": Value::Null, "devices": [] });
    }

    // Try rocminfo for version
    let version = detect_rocm_version();

    json!({
        "available": true,
        "version": version,
        "devices": devices,
    })
}

fn detect_rocm_version() -> Value {
    let cmd = Command::new("rocminfo");
    let Some(out) = run_cmd(cmd, 3000) else { return Value::Null; };
    for line in out.lines() {
        if line.contains("Runtime Version") {
            if let Some(ver) = line.split(':').nth(1) {
                let v = ver.trim().to_string();
                if !v.is_empty() {
                    return json!(v);
                }
            }
        }
    }
    Value::Null
}

fn detect_vulkan() -> Value {
    let mut cmd = Command::new("vulkaninfo");
    cmd.args(["--summary"]);
    let available = run_cmd(cmd, 2000).is_some();
    json!({ "available": available })
}

fn detect_directml() -> Value {
    #[cfg(target_os = "windows")]
    {
        // DirectML.dll ships in System32 on Windows 10 1903+ (build 18362+).
        // Any DX12-capable GPU on Win10+ can use DirectML for ML inference.
        let available =
            std::path::Path::new(r"C:\Windows\System32\DirectML.dll").exists()
            || std::path::Path::new(r"C:\Windows\SysWOW64\DirectML.dll").exists();
        return json!({ "available": available });
    }
    #[cfg(not(target_os = "windows"))]
    json!({ "available": false })
}
//
// WebNN (Web Neural Network API) routes inference to the best available
// accelerator: DirectML → GPU/NPU on Windows; CoreML/ANE on macOS Apple Silicon.

fn detect_webnn() -> Value {
    #[cfg(target_os = "windows")]
    {
        // WebNN is available in WebView2 on Windows when DirectML is present.
        let dm = std::path::Path::new(r"C:\Windows\System32\DirectML.dll").exists()
            || std::path::Path::new(r"C:\Windows\SysWOW64\DirectML.dll").exists();
        if dm {
            return json!({ "available": true, "backends": ["directml"] });
        }
        return json!({ "available": false, "backends": [] });
    }
    #[cfg(target_os = "macos")]
    {
        // WebNN uses CoreML backend on macOS. Apple Silicon gets ANE acceleration.
        let is_apple_silicon = detect_apple_silicon();
        let backends = if is_apple_silicon {
            json!(["coreml", "ane"])
        } else {
            json!(["coreml"])
        };
        return json!({ "available": true, "backends": backends });
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    json!({ "available": false, "backends": [] })
}

fn detect_npu() -> Value {
    let intel    = detect_intel_npu();
    let amd      = detect_amd_xdna();
    let apple    = detect_apple_ane();
    let qualcomm = detect_qualcomm_npu();
    json!({
        "intel":    intel,
        "amdXdna":  amd,
        "appleAne": apple,
        "qualcomm": qualcomm,
    })
}

// Qualcomm Hexagon NPU — Snapdragon X / Snapdragon Elite laptops
/// Query Win32_PnPEntity by PowerShell name filter. Returns {available, name} or false.
#[cfg(target_os = "windows")]
fn detect_windows_pnp(filter: &str, fallback_name: &str) -> Value {
    let mut cmd = Command::new("powershell");
    cmd.args([
        "-NoProfile", "-NonInteractive", "-Command",
        &format!(
            "Get-CimInstance Win32_PnPEntity | Where-Object {{ {filter} }} | \
             Select-Object Name | ConvertTo-Json -Compress"
        ),
    ]);
    let Some(out) = run_cmd(cmd, 5000) else { return json!({ "available": false }) };
    let Ok(parsed) = serde_json::from_str::<Value>(out.trim()) else {
        return json!({ "available": false });
    };
    let items = super::json_items(&parsed);
    if items.is_empty() { return json!({ "available": false }) }
    let name = items[0]["Name"].as_str().unwrap_or(fallback_name).trim().to_string();
    json!({ "available": true, "name": name })
}

// Intel NPU — "Intel AI Boost" (Core Ultra), "Intel VPU"
fn detect_intel_npu() -> Value {
    #[cfg(target_os = "windows")]
    return detect_windows_pnp(
        "$_.Name -like '*Intel*AI*' -or $_.Name -like '*Intel*NPU*' -or \
         $_.Name -like '*Intel*VPU*' -or $_.Name -like '*AI Boost*'",
        "Intel NPU",
    );
    #[cfg(target_os = "linux")]
    {
        // /dev/accel/accel* with vendor 0x8086
        if let Ok(dir) = std::fs::read_dir("/sys/class/accel") {
            for entry in dir.flatten() {
                if let Ok(vendor) = std::fs::read_to_string(entry.path().join("device/vendor")) {
                    if vendor.trim() == "0x8086" {
                        let name = std::fs::read_to_string(entry.path().join("device/uevent"))
                            .ok()
                            .and_then(|s| s.lines()
                                .find(|l| l.starts_with("DRIVER="))
                                .map(|l| l.trim_start_matches("DRIVER=").to_string()))
                            .unwrap_or_else(|| "Intel NPU".to_string());
                        return json!({ "available": true, "name": name });
                    }
                }
            }
        }
        if std::path::Path::new("/sys/bus/pci/drivers/intel_vpu").exists() {
            return json!({ "available": true, "name": "Intel VPU" });
        }
        json!({ "available": false })
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    json!({ "available": false })
}

// AMD XDNA / Ryzen AI
fn detect_amd_xdna() -> Value {
    #[cfg(target_os = "windows")]
    return detect_windows_pnp(
        "$_.Name -like '*AMD*IPU*' -or $_.Name -like '*Ryzen AI*' -or \
         $_.Name -like '*XDNA*' -or $_.Name -like '*AMD AI*'",
        "AMD XDNA",
    );
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new("/sys/bus/platform/drivers/amdxdna").exists() {
            return json!({ "available": true, "name": "AMD XDNA" });
        }
        if let Some(out) = run_cmd({ let mut c = Command::new("lsmod"); c }, 2000) {
            if out.lines().any(|l| l.starts_with("amdxdna")) {
                return json!({ "available": true, "name": "AMD XDNA" });
            }
        }
        json!({ "available": false })
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    json!({ "available": false })
}

// Qualcomm Hexagon NPU — Snapdragon X / Elite
fn detect_qualcomm_npu() -> Value {
    #[cfg(target_os = "windows")]
    return detect_windows_pnp(
        "$_.Name -like '*Qualcomm*NPU*' -or $_.Name -like '*Hexagon*' -or \
         $_.Name -like '*Snapdragon*NPU*'",
        "Qualcomm Hexagon NPU",
    );
    #[cfg(not(target_os = "windows"))]
    json!({ "available": false })
}

// Apple Neural Engine — present on all Apple Silicon (M1/M2/M3/M4)
fn detect_apple_ane() -> Value {
    #[cfg(target_os = "macos")]
    {
        let available = detect_apple_silicon();
        return json!({ "available": available });
    }
    #[cfg(not(target_os = "macos"))]
    json!({ "available": false })
}

#[cfg(target_os = "macos")]
fn detect_apple_silicon() -> bool {
    // sysctl hw.optional.ane_version returns 0 on Intel Macs, non-zero on Apple Silicon
    if let Some(out) = run_cmd(
        { let mut c = Command::new("sysctl"); c.args(["-n", "hw.optional.ane_version"]); c },
        1000
    ) {
        let v: u64 = out.trim().parse().unwrap_or(0);
        if v > 0 { return true; }
    }
    // Fallback: check chip name from system_profiler
    if let Some(out) = run_cmd(
        { let mut c = Command::new("sysctl"); c.args(["-n", "machdep.cpu.brand_string"]); c },
        1000
    ) {
        let s = out.trim().to_lowercase();
        if s.contains("apple m") { return true; }
    }
    false
}
