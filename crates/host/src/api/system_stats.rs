//! System statistics — CPU, memory, disks, processes, uptime.
//!
//! `sysinfo` snapshot lives in a process-wide mutex so CPU-usage deltas
//! have a baseline (first refresh always reports 0%).
//!
//! `Disks` live in their own collection (sysinfo 0.32+ split). The
//! collection is cached and refreshed in place per call.

use std::sync::LazyLock;

use parking_lot::{Mutex, MutexGuard};
use serde_json::{json, Value};
use smallvec::SmallVec;
use sysinfo::{Disks, ProcessRefreshKind, RefreshKind, System};

use crate::ipc::error::IpcError;

use crate::api::HostApi;
use crate::tynd;

/// Dispatch marker — sysinfo state lives in `SYS` below.
#[derive(Debug, Default)]
pub(crate) struct SystemStatsApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("systemStats", |method, args| dispatch(method, &args));

#[tynd::handler(api = "systemStats")]
impl SystemStatsApi {
    /// Aggregate CPU usage (%) plus core count and brand string.
    /// Usage is the mean across logical CPUs since the previous
    /// `cpu()` snapshot — the first call always reports 0% because
    /// sysinfo needs two refreshes to compute a delta.
    pub fn cpu() -> Result<Value, String> {
        cpu_impl()
    }

    /// Total / used / free physical and swap memory, in bytes.
    pub fn memory() -> Result<Value, String> {
        memory_impl()
    }

    /// Per-mount disk stats: total/available bytes and filesystem
    /// type. Refreshed in place per call from a cached `Disks` snapshot.
    pub fn disks() -> Result<Value, String> {
        disks_impl()
    }

    /// Paginated process listing sorted by ascending PID. `limit`
    /// defaults to 50 and is capped at 1000; `offset` skips from the
    /// front of the list.
    pub fn processes(limit: Option<u64>, offset: Option<u64>) -> Result<Value, String> {
        let (limit, offset) = normalize_process_page(limit, offset);
        processes_impl(limit, offset)
    }

    /// Host uptime in seconds (`{ "seconds": u64 }`).
    pub fn uptime() -> Result<Value, String> {
        System::uptime()
            .map(|seconds| json!({ "seconds": seconds }))
            .map_err(|e| stats_error("uptime", &e))
    }
}

/// Process-wide `System` snapshot. sysinfo computes CPU usage as a
/// delta between two refreshes — re-instantiating per call always
/// reports 0%. The mutex is short-lived (refresh + read) so contention
/// stays bounded even under concurrent OS calls. `LazyLock` builds the
/// `System` once on first access; sysinfo 0.40 reports platform
/// initialisation failures as `Err`, cached here as a message.
static SYS: LazyLock<Result<Mutex<System>, String>> = LazyLock::new(|| {
    System::new_with_specifics(RefreshKind::nothing())
        .map(Mutex::new)
        .map_err(|e| stats_error("init", &e))
});

/// Process-wide `Disks` snapshot. Re-listing disks rebuilds an internal
/// inode/device map; cache and refresh in place to avoid the per-call
/// rebuild cost on the IPC hot path.
static DISKS: LazyLock<Result<Mutex<Disks>, String>> = LazyLock::new(|| {
    Disks::new_with_refreshed_list()
        .map(Mutex::new)
        .map_err(|e| stats_error("disks", &e))
});

fn stats_error(method: &str, error: &sysinfo::Error) -> String {
    IpcError::new("systemStats", method, error).into_string()
}

fn lock_system() -> Result<MutexGuard<'static, System>, String> {
    SYS.as_ref().map(|sys| sys.lock()).map_err(Clone::clone)
}

fn cpu_impl() -> Result<Value, String> {
    let mut s = lock_system()?;
    s.refresh_cpu_usage();
    let cpus = s.cpus();
    // Aggregate usage = mean across logical CPUs. Per-core breakdown
    // would change the API shape; callers that want it can iterate
    // with a future `cpuPerCore()` method.
    let count = cpus.len();
    let cpu_rows: Vec<(f32, String)> = cpus
        .iter()
        .map(|cpu| (cpu.usage(), cpu.brand().to_string()))
        .collect();
    debug_assert_eq!(cpu_rows.len(), count);
    Ok(cpu_json(&cpu_rows))
}

fn memory_impl() -> Result<Value, String> {
    let mut s = lock_system()?;
    s.refresh_memory();
    Ok(memory_json(
        s.total_memory(),
        s.used_memory(),
        s.total_swap(),
        s.used_swap(),
    ))
}

fn disks_impl() -> Result<Value, String> {
    let mut disks = DISKS.as_ref().map_err(Clone::clone)?.lock();
    disks.refresh(true);
    let arr: Vec<Value> = disks
        .iter()
        .map(|d| {
            disk_json(
                d.name().to_string_lossy(),
                d.mount_point().to_string_lossy(),
                d.total_space(),
                d.available_space(),
                d.file_system().to_string_lossy(),
            )
        })
        .collect();
    Ok(Value::Array(arr))
}

fn processes_impl(limit: usize, offset: usize) -> Result<Value, String> {
    let mut s = lock_system()?;
    s.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );

    // Stable order = ascending PID. sysinfo's `processes()` returns a
    // HashMap; sort once then page. Use a `SmallVec` with a 64-PID inline
    // capacity so dev machines / lightly loaded boxes never spill to the
    // heap; busier hosts (>64 procs) transparently fall back to heap.
    let mut pids: SmallVec<[&sysinfo::Pid; 64]> = s.processes().keys().collect();
    pids.sort();

    let arr: Vec<Value> = pids
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(|pid| {
            let p = s.processes().get(pid)?;
            Some(process_json(
                pid.as_u32(),
                p.name().to_string_lossy(),
                p.cpu_usage(),
                p.memory(),
                p.parent().map(sysinfo::Pid::as_u32),
            ))
        })
        .collect();
    Ok(Value::Array(arr))
}

fn normalize_process_page(limit: Option<u64>, offset: Option<u64>) -> (usize, usize) {
    (
        limit.unwrap_or(50).min(1000) as usize,
        offset.unwrap_or(0) as usize,
    )
}

fn cpu_json<S: AsRef<str>>(cpus: &[(f32, S)]) -> Value {
    let count = cpus.len();
    let usage = if count == 0 {
        0.0_f32
    } else {
        cpus.iter().map(|(usage, _)| *usage).sum::<f32>() / count as f32
    };
    let brand = cpus
        .first()
        .map(|(_, brand)| brand.as_ref().to_string())
        .unwrap_or_default();
    json!({
        "usage": usage,
        "count": count,
        "brand": brand,
    })
}

fn memory_json(total: u64, used: u64, swap_total: u64, swap_used: u64) -> Value {
    json!({
        "total": total,
        "used": used,
        "free": total.saturating_sub(used),
        "swap_total": swap_total,
        "swap_used": swap_used,
    })
}

fn disk_json(
    name: impl AsRef<str>,
    mount: impl AsRef<str>,
    total: u64,
    available: u64,
    fs_type: impl AsRef<str>,
) -> Value {
    json!({
        "name": name.as_ref(),
        "mount": mount.as_ref(),
        "total": total,
        "available": available,
        "fs_type": fs_type.as_ref(),
    })
}

fn process_json(
    pid: u32,
    name: impl AsRef<str>,
    cpu: f32,
    memory: u64,
    parent_pid: Option<u32>,
) -> Value {
    json!({
        "pid": pid,
        "name": name.as_ref(),
        "cpu": cpu,
        "memory": memory,
        "parent_pid": parent_pid,
    })
}
