import os from "os";

export type Platform = "windows" | "macos" | "linux";
export type Arch     = "x64" | "arm64" | string;
export type Family   = "windows" | "unix";

function normalizePlatform(): Platform {
  if (process.platform === "darwin") return "macos";
  if (process.platform === "win32")  return "windows";
  return "linux";
}

export class OsInfo {
  readonly platform: Platform = normalizePlatform();
  readonly arch:     Arch     = process.arch;
  readonly family:   Family   = process.platform === "win32" ? "windows" : "unix";
  readonly eol:      string   = os.EOL;

  /** Kernel / OS release — e.g. `"10.0.22631"` or `"24.0.0"`. */
  version(): string { return os.release(); }
  /** BCP-47 — `"en-US"`, `"fr-FR"`, etc. */
  locale(): string { return Intl.DateTimeFormat().resolvedOptions().locale; }
  hostname(): string { return os.hostname(); }
  /** Seconds since OS boot. */
  uptime(): number { return os.uptime(); }
  /** Bytes. */
  totalMemory(): number { return os.totalmem(); }
  /** Bytes. */
  freeMemory(): number { return os.freemem(); }
  userInfo() { return os.userInfo(); }
}
