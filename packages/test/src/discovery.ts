import { lstatSync, readdirSync, readFileSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { Endpoint } from "./transport.ts";

/** One `<pid>.json` file the host writes when its debug socket is up. */
export interface DiscoveryEntry {
  pid: number;
  appId: string;
  endpoint: Endpoint;
  token: string;
  startedAt: string;
  /** Last modification time of the discovery file (epoch ms). */
  mtimeMs: number;
}

/** Directory where hosts publish their discovery files (mirrors crates/host/src/debug.rs). */
export function defaultDiscoveryDir(
  env: Record<string, string | undefined> = process.env,
  platform: NodeJS.Platform = process.platform,
  euid: number = effectiveUid(),
): string {
  const override = env.TYND_TEST_DISCOVERY_DIR;
  if (override) return override;
  if (platform === "win32") {
    const base = env.LOCALAPPDATA ?? join(homedir(), "AppData", "Local");
    return join(base, "tynd", "debug");
  }
  if (platform === "darwin") {
    return join(homedir(), "Library", "Application Support", "tynd", "debug");
  }
  if (env.XDG_RUNTIME_DIR) return join(env.XDG_RUNTIME_DIR, "tynd");
  // Literal /tmp, not os.tmpdir(): the host ignores TMPDIR (crates/host/src/debug.rs).
  return join("/tmp", `tynd-${euid}`);
}

function effectiveUid(): number {
  if (typeof process.geteuid === "function") return process.geteuid();
  return typeof process.getuid === "function" ? process.getuid() : 0;
}

/** Parse the JSON body of a discovery file. Returns `undefined` for anything unusable. */
export function parseDiscovery(text: string, mtimeMs = 0): DiscoveryEntry | undefined {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return;
  }
  if (typeof value !== "object" || value === null) return;
  const raw = value as Record<string, unknown>;
  const endpoint = raw.endpoint as Record<string, unknown> | undefined;
  const token = raw.cookie;
  if (typeof raw.pid !== "number" || typeof token !== "string" || !endpoint) return;
  let parsed: Endpoint;
  if (typeof endpoint.unix === "string" && endpoint.unix) {
    parsed = { unix: endpoint.unix };
  } else if (typeof endpoint.tcp === "number") {
    parsed = {
      tcp: endpoint.tcp,
      host: typeof endpoint.host === "string" ? endpoint.host : undefined,
    };
  } else {
    return;
  }
  return {
    pid: raw.pid,
    appId: typeof raw.appId === "string" ? raw.appId : "",
    endpoint: parsed,
    token,
    startedAt: typeof raw.startedAt === "string" ? raw.startedAt : "",
    mtimeMs,
  };
}

/**
 * The uid to enforce on discovery files, or `undefined` where ownership is not checked (Windows,
 * where the per-user directory is already ACL-protected).
 */
function enforcedUid(): number | undefined {
  if (process.platform === "win32" || typeof process.geteuid !== "function") return;
  return process.geteuid();
}

/**
 * Read every valid discovery file in `dir`. A missing directory yields an empty list. On Unix the
 * directory must be a real directory owned by the current user with mode 0700 (what the host
 * creates), and each file must be a regular file owned by that user; anything else is ignored so a
 * foreign user cannot plant a discovery file that redirects the test client and steals its token.
 */
export function readDiscoveryEntries(
  dir: string,
  uid: number | undefined = enforcedUid(),
): DiscoveryEntry[] {
  if (uid !== undefined && !isTrustedDir(dir, uid)) return [];
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return [];
  }
  const entries: DiscoveryEntry[] = [];
  for (const name of names) {
    if (!name.endsWith(".json")) continue;
    const path = join(dir, name);
    try {
      if (uid !== undefined && !isOwnedFile(path, uid)) continue;
      const entry = parseDiscovery(readFileSync(path, "utf8"), statSync(path).mtimeMs);
      if (entry) entries.push(entry);
    } catch {
      // The host may be rewriting or deleting the file; skip it.
    }
  }
  return entries;
}

function isTrustedDir(dir: string, uid: number): boolean {
  try {
    const info = lstatSync(dir);
    return info.isDirectory() && info.uid === uid && (info.mode & 0o777) === 0o700;
  } catch {
    return false;
  }
}

function isOwnedFile(path: string, uid: number): boolean {
  const info = lstatSync(path);
  return info.isFile() && info.uid === uid;
}

export interface SelectOptions {
  /** Ignore entries written before this time (epoch ms). */
  notBefore: number;
  /** The debug socket path the launch asked for (`TYND_DEBUG_SOCKET`), when honored by the launcher. */
  socket?: string;
  /** PID of the process that was spawned; `tynd dev` embeds it in its socket name. */
  launcherPid?: number;
  appId?: string;
  /** Liveness probe, injectable for tests. */
  isAlive?: (pid: number) => boolean;
}

/**
 * Pick the entry that belongs to a freshly launched app (restricted to `appId` when given).
 * When `socket` is given only an entry listening on exactly that socket qualifies: falling back to
 * another fresh entry would attach to a different app's host. Without `socket` the order is the
 * socket named after the launcher pid, then the newest entry.
 */
export function selectEntry(
  entries: DiscoveryEntry[],
  options: SelectOptions,
): DiscoveryEntry | undefined {
  const alive = options.isAlive ?? isProcessAlive;
  const fresh = entries
    .filter((entry) => entry.mtimeMs >= options.notBefore && alive(entry.pid))
    .filter((entry) => !options.appId || entry.appId === options.appId)
    .sort((left, right) => right.mtimeMs - left.mtimeMs);
  const unixPath = (entry: DiscoveryEntry) => ("unix" in entry.endpoint ? entry.endpoint.unix : "");
  if (options.socket) {
    const socket = options.socket;
    return fresh.find((entry) => unixPath(entry) === socket);
  }
  const rules: ((entry: DiscoveryEntry) => boolean)[] = [];
  if (options.launcherPid !== undefined) {
    const suffix = new RegExp(`-${options.launcherPid}(\\.sock)?$`);
    rules.push((entry) => suffix.test(unixPath(entry)));
  }
  for (const rule of rules) {
    const match = fresh.find(rule);
    if (match) return match;
  }
  return fresh[0];
}

export function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}
