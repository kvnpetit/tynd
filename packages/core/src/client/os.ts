import { osCall } from "./_internal.js"

export interface OsInfo {
  platform: "linux" | "macos" | "windows" | string
  arch: string
  family: string
  /** Short OS version (e.g. `"10.0.26200"` on Windows, `"14.5"` on macOS). */
  version: string | null
}

export const os = {
  info(): Promise<OsInfo> {
    return osCall("os", "info")
  },
  hostname(): Promise<string | null> {
    return osCall("os", "hostname")
  },
  /** BCP-47-ish locale tag from env vars (e.g. `"fr-FR"`, `"en-US"`). */
  locale(): Promise<string | null> {
    return osCall("os", "locale")
  },
  /** Whether the OS is currently in dark appearance. */
  isDarkMode(): Promise<boolean> {
    return osCall("os", "isDarkMode")
  },
  /** EOL character for the current OS (`"\r\n"` on Windows, `"\n"` otherwise). */
  eol(): Promise<string> {
    return osCall("os", "eol")
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
  desktopDir(): Promise<string | null> {
    return osCall("os", "desktopDir")
  },
  downloadsDir(): Promise<string | null> {
    return osCall("os", "downloadsDir")
  },
  documentsDir(): Promise<string | null> {
    return osCall("os", "documentsDir")
  },
  picturesDir(): Promise<string | null> {
    return osCall("os", "picturesDir")
  },
  musicDir(): Promise<string | null> {
    return osCall("os", "musicDir")
  },
  videoDir(): Promise<string | null> {
    return osCall("os", "videoDir")
  },
  exePath(): Promise<string | null> {
    return osCall("os", "exePath")
  },
  /** Directory of the currently running binary (resources packed next to it). */
  resourceDir(): Promise<string | null> {
    return osCall("os", "resourceDir")
  },
  cwd(): Promise<string | null> {
    return osCall("os", "cwd")
  },
  env(key: string): Promise<string | null> {
    return osCall("os", "env", { key })
  },
}
