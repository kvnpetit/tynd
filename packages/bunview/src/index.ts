export { BunviewApp, createApp } from "./app";
export type {
  AiCapabilities, AppConfig, AudioDevice, BatteryInfo, BunviewConfig, ClientCommands, CommandMap, CpuDetails, CpuUsage, DiskInfo, DisplayInfo, EventMap, FileFilter, GpuInfo, GpuUsageInfo, HardwareMonitorData, InferInput,
  InferOutput, MemoryInfo, MenuItem,
  MessageDialogOptions,
  FileAssociation, MonitorInfo, NetworkInterface, NetworkSpeed, OpenDirectoryOptions, OpenFileOptions, ProcessInfo, RamModule, RPCHandler, SaveFileOptions, SecondInstancePayload, SystemInfo, TemperatureInfo, TrayMenuItem, UsbDevice, UserInfo, WindowConfig, WindowHandle, WindowOptions, WindowPosition
} from "./types";

// Optional modules — prefer subpath imports for better tree-shaking
export * as autolaunch from "./autolaunch";
export * as shell from "./shell";
export * as autoUpdater from "./updater";
export { downloadFile } from "./http";
export type { DownloadProgress, DownloadOptions } from "./http";
export { serialize as bytesEncode, deserialize as bytesDecode } from "./binary";
export { Channel } from "./channel";

/**
 * Define a typed bunview project config.
 *
 * Pass the commands module as a generic to get **type-safe handler names**:
 *
 * @example
 * import type * as commands from "./backend/commands";
 *
 * export default defineConfig<typeof commands>({
 *   fileAssociations: [{ ext: "md", handler: "openFile" }],  // ← tsc verifies handler exists
 * });
 */
export function defineConfig<TCommands extends import("./types").CommandMap = import("./types").CommandMap>(
  config: import("./types").BunviewConfig<TCommands>,
): import("./types").BunviewConfig<TCommands> {
  return config;
}
