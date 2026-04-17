import { osCall } from "./_internal.js"

export interface OsInfo {
  platform: "linux" | "macos" | "windows" | string
  arch: string
  family: string
}

export const os = {
  info(): Promise<OsInfo> {
    return osCall("os", "info")
  },
  homeDir(): Promise<string | null> {
    return osCall("os", "homeDir")
  },
  tmpDir(): Promise<string> {
    return osCall("os", "tmpDir") as Promise<string>
  },
  configDir(): Promise<string | null> {
    return osCall("os", "configDir")
  },
  dataDir(): Promise<string | null> {
    return osCall("os", "dataDir")
  },
  cacheDir(): Promise<string | null> {
    return osCall("os", "cacheDir")
  },
  exePath(): Promise<string | null> {
    return osCall("os", "exePath")
  },
  cwd(): Promise<string | null> {
    return osCall("os", "cwd")
  },
  env(key: string): Promise<string | null> {
    return osCall("os", "env", { key })
  },
}
