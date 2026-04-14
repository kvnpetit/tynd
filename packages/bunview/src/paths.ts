import path from "path";
import os from "os";
import fs from "fs";

const PLATFORM = process.platform;

function ensureDir(dir: string): string {
  try { fs.mkdirSync(dir, { recursive: true }); } catch {}
  return dir;
}

/**
 * Cross-platform standard directories, XDG-compliant on Linux,
 * Apple HIG-compliant on macOS, Known Folders on Windows.
 */
export class AppPaths {
  constructor(private readonly appName: string) {}

  /** Persistent user application data. */
  data(): string {
    if (PLATFORM === "win32") {
      return ensureDir(path.join(process.env.APPDATA ?? path.join(os.homedir(), "AppData", "Roaming"), this.appName));
    }
    if (PLATFORM === "darwin") {
      return ensureDir(path.join(os.homedir(), "Library", "Application Support", this.appName));
    }
    const xdg = process.env.XDG_DATA_HOME ?? path.join(os.homedir(), ".local", "share");
    return ensureDir(path.join(xdg, this.appName));
  }

  /** User configuration. */
  config(): string {
    if (PLATFORM === "win32") return this.data();
    if (PLATFORM === "darwin") {
      return ensureDir(path.join(os.homedir(), "Library", "Preferences", this.appName));
    }
    const xdg = process.env.XDG_CONFIG_HOME ?? path.join(os.homedir(), ".config");
    return ensureDir(path.join(xdg, this.appName));
  }

  /** Transient cache (may be cleared by the OS). */
  cache(): string {
    if (PLATFORM === "win32") {
      return ensureDir(path.join(process.env.LOCALAPPDATA ?? path.join(os.homedir(), "AppData", "Local"), this.appName, "Cache"));
    }
    if (PLATFORM === "darwin") {
      return ensureDir(path.join(os.homedir(), "Library", "Caches", this.appName));
    }
    const xdg = process.env.XDG_CACHE_HOME ?? path.join(os.homedir(), ".cache");
    return ensureDir(path.join(xdg, this.appName));
  }

  /** App log files. */
  logs(): string {
    if (PLATFORM === "win32") return ensureDir(path.join(this.data(), "logs"));
    if (PLATFORM === "darwin") {
      return ensureDir(path.join(os.homedir(), "Library", "Logs", this.appName));
    }
    const xdg = process.env.XDG_STATE_HOME ?? path.join(os.homedir(), ".local", "state");
    return ensureDir(path.join(xdg, this.appName, "logs"));
  }

  /** OS temporary directory (shared, not app-specific). */
  temp(): string { return os.tmpdir(); }

  /** User home directory. */
  home(): string { return os.homedir(); }

  /** Current running executable. */
  executable(): string { return process.execPath; }
  downloads(): string { return path.join(os.homedir(), "Downloads"); }
  documents(): string { return path.join(os.homedir(), "Documents"); }
  desktop():   string { return path.join(os.homedir(), "Desktop"); }
  pictures():  string { return path.join(os.homedir(), "Pictures"); }
  music():     string { return path.join(os.homedir(), "Music"); }
  videos():    string {
    return path.join(os.homedir(), PLATFORM === "darwin" ? "Movies" : "Videos");
  }
}
