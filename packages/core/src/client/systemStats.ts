import { osCall } from "./_internal";

/** Aggregate CPU snapshot — mean usage across logical CPUs. */
export interface CpuStats {
  /** Mean usage across logical CPUs, percentage in `[0, 100]`. */
  usage: number;
  /** Number of logical CPUs. */
  count: number;
  /** Vendor / model string of the first CPU (best-effort). */
  brand: string;
}

/** Memory + swap snapshot in bytes. */
export interface MemoryStats {
  total: number;
  used: number;
  free: number;
  swap_total: number;
  swap_used: number;
}

/** Mounted-disk descriptor returned by {@link systemStats.disks}. */
export interface DiskInfo {
  name: string;
  mount: string;
  total: number;
  available: number;
  fs_type: string;
}

/** Single-process descriptor returned by {@link systemStats.processes}. */
export interface ProcessInfo {
  pid: number;
  name: string;
  /** Instantaneous CPU usage, percentage in `[0, 100]`. */
  cpu: number;
  /** Resident memory in bytes. */
  memory: number;
  parent_pid: number | null;
}

/** Host uptime in whole seconds. */
export interface UptimeStats {
  seconds: number;
}

/**
 * `systemStats` — sample CPU, memory, disks, processes, and uptime.
 *
 * Backed by `sysinfo`. The host keeps a single `System` instance behind
 * a mutex so CPU usage is computed as a delta between successive
 * `cpu()` calls (the very first call returns a meaningful baseline rather
 * than a flat 0%).
 */
export const systemStats = {
  /** Aggregate CPU usage + core count + brand. */
  cpu(): Promise<CpuStats> {
    return osCall("systemStats", "cpu", {});
  },
  /** RAM + swap totals / usage in bytes. */
  memory(): Promise<MemoryStats> {
    return osCall("systemStats", "memory", {});
  },
  /** Mounted disks with capacity and filesystem type. */
  disks(): Promise<DiskInfo[]> {
    return osCall("systemStats", "disks", {});
  },
  /**
   * Page through running processes (sorted by ascending PID).
   *
   * @param limit  Defaults to `50` on the host side, capped at `1000` to
   *               keep a single response under the IPC reasonable-payload
   *               threshold.
   * @param offset Skip the first N entries. Defaults to `0`.
   */
  processes(limit?: number, offset?: number): Promise<ProcessInfo[]> {
    const args: Record<string, number> = {};
    if (limit !== undefined) args.limit = limit;
    if (offset !== undefined) args.offset = offset;
    return osCall("systemStats", "processes", args);
  },
  /** Host uptime in seconds. */
  uptime(): Promise<UptimeStats> {
    return osCall("systemStats", "uptime", {});
  },
};
