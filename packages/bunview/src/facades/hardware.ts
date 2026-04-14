import type { WebviewHost } from "../host";
import type {
  SystemInfo, CpuUsage, MemoryInfo, BatteryInfo, DiskInfo, NetworkInterface,
  GpuUsageInfo, TemperatureInfo, UsbDevice, AiCapabilities, NetworkSpeed,
  ProcessInfo, UserInfo, AudioDevice, DisplayInfo, CpuDetails, RamModule,
  HardwareMonitorData,
} from "../types";

export function hardwareFacade(host: WebviewHost | undefined) {
  const err = () => Promise.reject(new Error("[bunview] host not ready"));
  return {
    getSystemInfo:     (): Promise<SystemInfo>         => host?.getSystemInfo()     ?? err(),
    getCpuUsage:       (): Promise<CpuUsage>           => host?.getCpuUsage()       ?? err(),
    getMemoryInfo:     (): Promise<MemoryInfo>         => host?.getMemoryInfo()     ?? err(),
    getBatteryInfo:    (): Promise<BatteryInfo>        => host?.getBatteryInfo()    ?? err(),
    getDiskInfo:       (): Promise<DiskInfo[]>         => host?.getDiskInfo()       ?? err(),
    getNetworkInfo:    (): Promise<NetworkInterface[]> => host?.getNetworkInfo()    ?? err(),
    getGpuUsage:       (): Promise<GpuUsageInfo[]>     => host?.getGpuUsage()       ?? err(),
    getTemperature:    (): Promise<TemperatureInfo>    => host?.getTemperature()    ?? err(),
    getUsbDevices:     (): Promise<UsbDevice[]>        => host?.getUsbDevices()     ?? err(),
    getAiCapabilities: (): Promise<AiCapabilities>     => host?.getAiCapabilities() ?? err(),
    getCpuDetails:     (): Promise<CpuDetails>         => host?.getCpuDetails()     ?? err(),
    getRamDetails:     (): Promise<RamModule[]>        => host?.getRamDetails()     ?? err(),
    /** Blocks ~1s for sampling. */
    getNetworkSpeed:   (): Promise<NetworkSpeed[]>     => host?.getNetworkSpeed()   ?? err(),
    /** Top 50 by CPU. */
    getProcessList:    (): Promise<ProcessInfo[]>      => host?.getProcessList()    ?? err(),
    getUsers:          (): Promise<UserInfo[]>         => host?.getUsers()          ?? err(),
    getAudioDevices:   (): Promise<AudioDevice[]>      => host?.getAudioDevices()   ?? err(),
    getDisplayInfo:    (): Promise<DisplayInfo[]>      => host?.getDisplayInfo()    ?? err(),
    /** Emits `hwMonitorUpdate`. Min 250ms. */
    startMonitoring: (intervalMs = 1000) => host?.startHwMonitor(intervalMs),
    stopMonitoring:  ()                  => host?.stopHwMonitor(),
    onUpdate: (cb: (data: HardwareMonitorData) => void) => {
      host?.on("hwMonitorUpdate", (evt: any) => cb({
        cpu:          evt.cpu,
        memory:       evt.memory,
        temperatures: evt.temperatures,
        networkSpeed: evt.networkSpeed ?? [],
        gpu:          evt.gpu,
        timestamp:    evt.timestamp,
      }));
    },
  };
}
