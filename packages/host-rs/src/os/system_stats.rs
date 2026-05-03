//! System statistics — CPU, memory, disks, processes, uptime.
//!
//! Backed by `sysinfo`, which keeps an internal snapshot that callers
//! refresh selectively. We hold a single process-wide `System` instance
//! behind a mutex so the snapshot persists across calls (required for
//! CPU-usage deltas to be meaningful — the first refresh always reports
//! 0% because there's no baseline).
//!
//! Disks live in their own `Disks` collection (sysinfo 0.32 split system
//! data from disk/network), recomputed on each call since they're cheap.

use std::sync::LazyLock;

use parking_lot::Mutex;
use serde_json::{json, Value};
use sysinfo::{Disks, ProcessRefreshKind, RefreshKind, System};

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "cpu" => Ok(cpu()),
        "memory" => Ok(memory()),
        "disks" => Ok(disks()),
        "processes" => Ok(processes(args)),
        "uptime" => Ok(json!({ "seconds": System::uptime() })),
        _ => Err(format!("systemStats.{method}: unknown method")),
    }
}

/// Process-wide `System` snapshot. sysinfo computes CPU usage as a
/// delta between two refreshes — re-instantiating per call always
/// reports 0%. The mutex is short-lived (refresh + read) so contention
/// stays bounded even under concurrent OS calls. `LazyLock` builds the
/// `System` once on first access.
static SYS: LazyLock<Mutex<System>> =
    LazyLock::new(|| Mutex::new(System::new_with_specifics(RefreshKind::new())));

fn cpu() -> Value {
    let mut s = SYS.lock();
    s.refresh_cpu_usage();
    let cpus = s.cpus();
    // Aggregate usage = mean across logical CPUs. Per-core breakdown
    // would change the API shape; callers that want it can iterate
    // with a future `cpuPerCore()` method.
    let count = cpus.len();
    let usage = if count == 0 {
        0.0_f32
    } else {
        cpus.iter().map(sysinfo::Cpu::cpu_usage).sum::<f32>() / count as f32
    };
    let brand = cpus
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();
    json!({
        "usage": usage,
        "count": count,
        "brand": brand,
    })
}

fn memory() -> Value {
    let mut s = SYS.lock();
    s.refresh_memory();
    let total = s.total_memory();
    let used = s.used_memory();
    json!({
        "total": total,
        "used": used,
        "free": total.saturating_sub(used),
        "swap_total": s.total_swap(),
        "swap_used": s.used_swap(),
    })
}

fn disks() -> Value {
    let disks = Disks::new_with_refreshed_list();
    let arr: Vec<Value> = disks
        .iter()
        .map(|d| {
            json!({
                "name": d.name().to_string_lossy(),
                "mount": d.mount_point().to_string_lossy(),
                "total": d.total_space(),
                "available": d.available_space(),
                "fs_type": d.file_system().to_string_lossy(),
            })
        })
        .collect();
    Value::Array(arr)
}

/// Process listing with simple pagination.
///
/// `limit` defaults to 50 and is capped at 1000 — listing every process
/// on a busy host can produce tens of MB of JSON, which the IPC layer
/// will happily ship but most frontends can't render. `offset` skips
/// from the front of the (PID-sorted) list.
fn processes(args: &Value) -> Value {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .min(1000) as usize;
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;

    let mut s = SYS.lock();
    s.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_cpu().with_memory(),
    );

    // Stable order = ascending PID. sysinfo's `processes()` returns a
    // HashMap; sort once then page.
    let mut pids: Vec<&sysinfo::Pid> = s.processes().keys().collect();
    pids.sort();

    let arr: Vec<Value> = pids
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(|pid| {
            let p = s.processes().get(pid)?;
            Some(json!({
                "pid": pid.as_u32(),
                "name": p.name().to_string_lossy(),
                "cpu": p.cpu_usage(),
                "memory": p.memory(),
                "parent_pid": p.parent().map(sysinfo::Pid::as_u32),
            }))
        })
        .collect();
    Value::Array(arr)
}
