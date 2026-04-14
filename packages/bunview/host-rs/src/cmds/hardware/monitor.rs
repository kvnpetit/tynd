// Auto-generated module split — edit freely.

use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use serde_json::{json, Value};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System, Networks};

use crate::ipc;
use super::{run_cmd, round1};
use super::devices::temperature_impl;
use super::gpu::try_nvidia_usage;
#[cfg(target_os = "linux")]
use super::gpu::try_rocm_gpu_usage;

static MONITOR_ACTIVE: AtomicBool = AtomicBool::new(false);
static MONITOR_INTERVAL: AtomicU64 = AtomicU64::new(1000);

pub fn start_hw_monitor(interval_ms: u64) {
    let interval = interval_ms.max(250);
    MONITOR_INTERVAL.store(interval, Ordering::SeqCst);

    // If already running, just update interval
    if MONITOR_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(|| {
        let mut sys = System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::new().with_cpu_usage().with_frequency())
                .with_memory(MemoryRefreshKind::everything()),
        );

        // Cache GPU tool availability to avoid re-probing every iteration
        let nvidia_available = {
            let mut cmd = Command::new("nvidia-smi");
            cmd.args(["--query-gpu=name", "--format=csv,noheader,nounits"]);
            run_cmd(cmd, 2000).is_some()
        };
        let rocm_available = !nvidia_available && {
            let mut cmd = Command::new("rocm-smi");
            cmd.args(["--showproductname", "--json"]);
            run_cmd(cmd, 2000).is_some()
        };

        // Network speed tracking: store last totals + time
        let mut last_nets: HashMap<String, (u64, u64)> = HashMap::new();
        let mut last_net_time = std::time::Instant::now();

        // Initial CPU sample (need two for meaningful %)
        sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());

        loop {
            if !MONITOR_ACTIVE.load(Ordering::SeqCst) {
                break;
            }

            let interval_ms = MONITOR_INTERVAL.load(Ordering::SeqCst);
            let loop_start = std::time::Instant::now();

            // === Network speed (delta since last iteration) ===
            let nets_snapshot: Vec<(String, u64, u64)> = {
                let nets = Networks::new_with_refreshed_list();
                nets.iter()
                    .map(|(n, d)| (n.clone(), d.total_received(), d.total_transmitted()))
                    .collect()
            };
            let elapsed_secs = loop_start.duration_since(last_net_time).as_secs_f64().max(0.001);
            let net_speed: Vec<Value> = nets_snapshot.iter().map(|(name, rx, tx)| {
                let (prev_rx, prev_tx) = last_nets.get(name).copied().unwrap_or((*rx, *tx));
                let rx_bps = (rx.saturating_sub(prev_rx) as f64 / elapsed_secs) as u64;
                let tx_bps = (tx.saturating_sub(prev_tx) as f64 / elapsed_secs) as u64;
                json!({
                    "name": name,
                    "rxBytesPerSec": rx_bps,
                    "txBytesPerSec": tx_bps,
                    "rxMbps": round1(rx_bps as f64 * 8.0 / 1_000_000.0),
                    "txMbps": round1(tx_bps as f64 * 8.0 / 1_000_000.0),
                })
            }).collect();
            for (name, rx, tx) in &nets_snapshot {
                last_nets.insert(name.clone(), (*rx, *tx));
            }
            last_net_time = loop_start;

            // Two-sample CPU measurement
            sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());
            std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
            sys.refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage().with_frequency());
            sys.refresh_memory();

            let cores: Vec<Value> = sys
                .cpus()
                .iter()
                .map(|c| json!({
                    "usage": round1(c.cpu_usage() as f64),
                    "frequency": c.frequency(),
                }))
                .collect();

            let cpu_payload = json!({
                "global": round1(sys.global_cpu_usage() as f64),
                "cores": cores,
            });

            let memory_payload = json!({
                "total":     sys.total_memory(),
                "used":      sys.used_memory(),
                "available": sys.available_memory(),
                "free":      sys.free_memory(),
            });

            // Temperatures — reuse the same logic as get_temperature()
            let temp_payload = temperature_impl();

            // GPU: nvidia-smi (NVIDIA) or rocm-smi (AMD) — skipped if neither present
            let gpu_payload: Option<Value> = if nvidia_available {
                try_nvidia_usage()
            } else if rocm_available {
                #[cfg(target_os = "linux")]
                { try_rocm_gpu_usage() }
                #[cfg(not(target_os = "linux"))]
                { None }
            } else {
                None
            };

            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);

            let mut event = json!({
                "type": "hwMonitorUpdate",
                "cpu": cpu_payload,
                "memory": memory_payload,
                "temperatures": temp_payload,
                "networkSpeed": net_speed,
                "timestamp": timestamp,
            });

            if let Some(gpu) = gpu_payload {
                event["gpu"] = gpu;
            }

            ipc::emit(event);

            // Sleep for remainder of interval (subtract time already spent on CPU sampling)
            let spent = sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.as_millis() as u64;
            let remaining = interval_ms.saturating_sub(spent);
            if remaining > 0 {
                std::thread::sleep(Duration::from_millis(remaining));
            }
        }
    });
}

pub fn stop_hw_monitor() {
    MONITOR_ACTIVE.store(false, Ordering::SeqCst);
}
