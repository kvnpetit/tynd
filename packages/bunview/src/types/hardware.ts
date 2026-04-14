export interface GpuInfo {
  name: string;
  /** Bytes. */
  vram?: number;
  type: "discrete" | "integrated" | "unknown";
}

export interface SystemInfo {
  cpu: {
    name: string;
    physicalCores: number;
    logicalCores: number;
    arch: string;
  };
  memory: { total: number };
  gpus: GpuInfo[];
  os: {
    name: string;
    version: string;
    kernel: string;
  };
}

/** All usage values 0-100. Frequencies in MHz. */
export interface CpuUsage {
  global: number;
  cores: Array<{ usage: number; frequency: number }>;
}

/** All values in bytes. */
export interface MemoryInfo {
  total: number;
  used: number;
  available: number;
  free: number;
  totalSwap: number;
  usedSwap: number;
}

export interface BatteryInfo {
  present: boolean;
  /** 0-100%. */
  level?: number;
  charging?: boolean;
  pluggedIn?: boolean;
  /** 0-100%. */
  health?: number;
  /** Minutes. */
  timeRemaining?: number;
}

export interface DiskInfo {
  name: string;
  label: string;
  filesystem: string;
  type: "SSD" | "HDD" | "Unknown";
  /** Bytes. */
  totalSpace: number;
  availableSpace: number;
  usedSpace: number;
  removable: boolean;
}

export interface NetworkInterface {
  name: string;
  mac: string;
  /** IPv4 + IPv6 addresses assigned to this interface. */
  ips: string[];
  /** Bytes received since boot. */
  received: number;
  transmitted: number;
  up: boolean;
}

export interface GpuUsageInfo {
  name: string;
  /** 0-100. */
  utilizationPercent?: number;
  /** Bytes. */
  vramUsed?: number;
  vramTotal?: number;
  temperatureCelsius?: number;
  powerWatts?: number;
}

/** CPU in °C average, components in °C each. */
export interface TemperatureInfo {
  cpu?: number;
  components: Array<{ label: string; temp: number }>;
}

export interface UsbDevice {
  name: string;
  manufacturer?: string;
  class?: string;
  vendorId?: string;
  productId?: string;
}

export interface AiCapabilities {
  cuda: {
    available: boolean;
    version?: string;
    devices: Array<{ name: string; computeCapability?: string; vram?: number }>;
  };
  rocm: {
    available: boolean;
    version?: string;
    devices: Array<{ name: string }>;
  };
  vulkan: { available: boolean };
  metal:  { available: boolean };
  /** Windows DX12 ML API — backend for WebNN on Windows. */
  directml: { available: boolean };
  /**
   * With `hardwareAcceleration: true`, bunview routes WebNN inference to the
   * best accelerator: DirectML (Windows) or CoreML/ANE (Apple Silicon).
   */
  webnn: {
    available: boolean;
    backends: string[];
  };
  npu: {
    /** Intel AI Boost / VPU — Core Ultra / Meteor Lake+. */
    intel?:    { available: boolean; name?: string };
    /** Apple Neural Engine — all Apple Silicon. */
    appleAne?: { available: boolean };
    /** AMD XDNA / Ryzen AI. */
    amdXdna?:  { available: boolean; name?: string };
    /** Qualcomm Hexagon — Snapdragon X / Elite. */
    qualcomm?: { available: boolean; name?: string };
  };
}

export interface NetworkSpeed {
  name: string;
  rxBytesPerSec: number;
  txBytesPerSec: number;
  rxMbps: number;
  txMbps: number;
}

export interface ProcessInfo {
  pid: number;
  name: string;
  /** % — can exceed 100 on multi-core (100 = 1 full core). */
  cpuUsage: number;
  memoryBytes: number;
  status: string;
}

export interface UserInfo {
  name: string;
}

export interface AudioDevice {
  name: string;
  type: "input" | "output";
  manufacturer?: string;
  status?: string;
}

export interface DisplayInfo {
  name: string;
  width?: number;
  height?: number;
  refreshRate?: number;
  primary?: boolean;
}

export interface RamModule {
  capacityBytes: number;
  /** "DDR4", "DDR5", "LPDDR5", etc. */
  type: string;
  speedMhz?: number;
  manufacturer?: string;
  partNumber?: string;
}

export interface CpuDetails {
  name: string;
  vendor?: string;
  l2CacheKb?: number;
  l3CacheKb?: number;
  maxClockMhz?: number;
  physicalCores?: number;
  logicalCores?: number;
}

export interface HardwareMonitorData {
  cpu: { global: number; cores: Array<{ usage: number; frequency: number }> };
  memory: { total: number; used: number; available: number; free: number };
  temperatures: { cpu?: number; components: Array<{ label: string; temp: number }> };
  networkSpeed: NetworkSpeed[];
  /** Present only when nvidia-smi or rocm-smi is detected. */
  gpu?: GpuUsageInfo[];
  timestamp: number;
}

export interface MonitorInfo {
  x: number; y: number; width: number; height: number; primary: boolean;
}
