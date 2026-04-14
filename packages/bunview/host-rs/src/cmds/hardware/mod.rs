// Auto-generated module split — edit freely.

mod system;
mod network;
mod gpu;
mod ai;
mod devices;
mod monitor;

pub use system::{
    get_system_info, get_cpu_usage, get_memory_info, get_battery_info,
    get_disk_info, get_process_list, get_users, get_cpu_details, get_ram_details,
};
pub use network::{get_network_info, get_network_speed};
pub use gpu::get_gpu_usage;
pub use ai::get_ai_capabilities;
pub use devices::{get_temperature, get_usb_devices, get_audio_devices, get_display_info};
pub use monitor::{start_hw_monitor, stop_hw_monitor};

use std::process::Command;
use std::time::Duration;
use serde_json::Value;

use crate::ipc;

/// Spawns a subprocess on a thread; returns stdout within `timeout_ms` or None.
pub(super) fn run_cmd(mut cmd: Command, timeout_ms: u64) -> Option<String> {
    use std::sync::mpsc;

    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let result = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .and_then(|out| {
                if out.status.success() {
                    String::from_utf8(out.stdout).ok()
                } else {
                    // nvidia-smi may return partial data with non-zero exit.
                    String::from_utf8(out.stdout)
                        .ok()
                        .filter(|s| !s.trim().is_empty())
                }
            });
        let _ = tx.send(result);
    });

    rx.recv_timeout(Duration::from_millis(timeout_ms))
        .unwrap_or(None)
}

pub(super) fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Spawn a worker thread that computes a JSON value and emits it as an IPC response.
/// Every `get_*` hardware entry point follows this pattern.
pub(super) fn spawn_response<F>(id: String, f: F)
where
    F: FnOnce() -> Value + Send + 'static,
{
    std::thread::spawn(move || {
        let result = f();
        ipc::emit_response(&id, result);
    });
}

/// Normalise `ConvertTo-Json -Compress` output (single object / array / null)
/// to a slice of references. Avoids the repeated `is_array ? arr : [&v]` dance.
#[allow(dead_code)] // unused on some platforms
pub(super) fn json_items(parsed: &Value) -> Vec<&Value> {
    if let Some(arr) = parsed.as_array() {
        arr.iter().collect()
    } else if !parsed.is_null() {
        vec![parsed]
    } else {
        Vec::new()
    }
}
