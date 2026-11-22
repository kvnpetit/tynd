import { osCall } from "./_internal";

/** OS identification returned by `os.info`. */
export interface OsInfo {
  platform: "linux" | "macos" | "windows" | string;
  arch: string;
  family: string;
  /** Short OS version (e.g. `"10.0.26200"` on Windows, `"14.5"` on macOS). */
  version: string | null;
}

/** Device whose capture permission `os.getMediaAccessStatus` / `os.askForMediaAccess` query. */
export type MediaAccessKind = "camera" | "microphone" | "screen";

/**
 * Capture permission state. `"not-determined"`: the user was never asked
 * (macOS); `"restricted"`: blocked by a policy the user cannot change
 * (macOS parental controls / MDM); `"unknown"`: the OS does not report it.
 */
export type MediaAccessStatus = "not-determined" | "granted" | "denied" | "restricted" | "unknown";

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
  /**
   * Region the user picked in the OS settings: an ISO 3166 code (`"FR"`,
   * `"US"`) or a UN M.49 area code (`"419"`). Can differ from the locale's
   * region. `null` when unknown.
   */
  region(): Promise<string | null> {
    return osCall("os", "region");
  },
  /** Preferred UI languages as BCP-47 tags, most preferred first (e.g. `["fr-FR", "en-US"]`). */
  languages(): Promise<string[]> {
    return osCall("os", "languages");
  },
  /** IANA time zone of the system (e.g. `"Europe/Paris"`), `null` when unknown. */
  timezone(): Promise<string | null> {
    return osCall("os", "timezone");
  },
  /** Whether the user's time format is 24-hour; `null` when the OS does not tell. */
  uses24HourClock(): Promise<boolean | null> {
    return osCall("os", "uses24HourClock");
  },
  /**
   * Capture permission of the app. macOS reads the privacy (TCC) state;
   * for `"screen"` it can only tell `"granted"` from `"denied"`. Windows
   * reads the Settings > Privacy switches for camera and microphone
   * (`"screen"` is always `"granted"`). Linux reports `"unknown"`: the
   * desktop portal asks on each capture.
   */
  getMediaAccessStatus(kind: MediaAccessKind): Promise<MediaAccessStatus> {
    return osCall("os", "getMediaAccessStatus", { kind });
  },
  /**
   * macOS: show the system permission prompt when the user has not decided
   * yet, and resolve to whether access is granted (the prompt appears once;
   * later calls report the stored decision). For `"screen"` the first call
   * opens the Screen Recording prompt and the grant takes effect after the
   * app restarts. Rejects on Windows and Linux, which have no such prompt.
   */
  askForMediaAccess(kind: MediaAccessKind): Promise<boolean> {
    return osCall("os", "askForMediaAccess", { kind });
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
  /**
   * The app's own data directory, `<dataDir>/<app>`, where `<app>` is the app
   * identifier (`bundle.identifier`; the executable name when unset). Created
   * on first call. Inside the default fs scope, like the four below.
   */
  appDataDir(): Promise<string> {
    return osCall("os", "appDataDir") as Promise<string>;
  },
  /** The app's own config directory, `<configDir>/<app>`. Created on first call. */
  appConfigDir(): Promise<string> {
    return osCall("os", "appConfigDir") as Promise<string>;
  },
  /** The app's own cache directory, `<cacheDir>/<app>`. Created on first call. */
  appCacheDir(): Promise<string> {
    return osCall("os", "appCacheDir") as Promise<string>;
  },
  /** The app's own temp directory, `<tmpDir>/<app>`. Created on first call. */
  appTempDir(): Promise<string> {
    return osCall("os", "appTempDir") as Promise<string>;
  },
  /** Directory of the default `log` file, `<cacheDir>/<app>/logs`. Created on first call. */
  appLogDir(): Promise<string> {
    return osCall("os", "appLogDir") as Promise<string>;
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
