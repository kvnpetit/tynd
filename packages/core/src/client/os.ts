import { osCall } from "./_internal";

/** OS identification returned by `os.info`. */
export interface OsInfo {
  platform: "linux" | "macos" | "windows" | string;
  arch: string;
  family: string;
  /** Short OS version (e.g. `"10.0.26200"` on Windows, `"14.5"` on macOS). */
  version: string | null;
}

/** Frontend bindings for the host's `os` API (platform info, locale, paths, env). */
export const os = {
  /** Read platform/arch/family/version metadata. */
  info(): Promise<OsInfo> {
    return osCall("os", "info");
  },
  /** OS-reported hostname or `null` when unavailable. */
  hostname(): Promise<string | null> {
    return osCall("os", "hostname");
  },
  /** BCP-47-ish locale tag from env vars (e.g. `"fr-FR"`, `"en-US"`). */
  locale(): Promise<string | null> {
    return osCall("os", "locale");
  },
  /** Whether the OS is currently in dark appearance. */
  isDarkMode(): Promise<boolean> {
    return osCall("os", "isDarkMode");
  },
  /** EOL character for the current OS (`"\r\n"` on Windows, `"\n"` otherwise). */
  eol(): Promise<string> {
    return osCall("os", "eol");
  },
  /** Path to the user's home directory. */
  homeDir(): Promise<string | null> {
    return osCall("os", "homeDir");
  },
  /** OS temp directory (always present). */
  tmpDir(): Promise<string> {
    return osCall("os", "tmpDir") as Promise<string>;
  },
  /** XDG/AppData config root. */
  configDir(): Promise<string | null> {
    return osCall("os", "configDir");
  },
  /** XDG/AppData data root. */
  dataDir(): Promise<string | null> {
    return osCall("os", "dataDir");
  },
  /** XDG/AppData cache root. */
  cacheDir(): Promise<string | null> {
    return osCall("os", "cacheDir");
  },
  /** User's Desktop directory. */
  desktopDir(): Promise<string | null> {
    return osCall("os", "desktopDir");
  },
  /** User's Downloads directory. */
  downloadsDir(): Promise<string | null> {
    return osCall("os", "downloadsDir");
  },
  /** User's Documents directory. */
  documentsDir(): Promise<string | null> {
    return osCall("os", "documentsDir");
  },
  /** User's Pictures directory. */
  picturesDir(): Promise<string | null> {
    return osCall("os", "picturesDir");
  },
  /** User's Music directory. */
  musicDir(): Promise<string | null> {
    return osCall("os", "musicDir");
  },
  /** User's Videos directory. */
  videoDir(): Promise<string | null> {
    return osCall("os", "videoDir");
  },
  /** Absolute path of the running binary. */
  exePath(): Promise<string | null> {
    return osCall("os", "exePath");
  },
  /** Directory of the currently running binary (resources packed next to it). */
  resourceDir(): Promise<string | null> {
    return osCall("os", "resourceDir");
  },
  /** Current working directory of the host process. */
  cwd(): Promise<string | null> {
    return osCall("os", "cwd");
  },
  /** Read a single environment variable from the host process. */
  env(key: string): Promise<string | null> {
    return osCall("os", "env", { key });
  },
};
