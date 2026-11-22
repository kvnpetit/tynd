/**
 * Local crash reports (`app.crashReporter`) and renderer process loss
 * (`app.on("render-process-gone")`). Reports stay on disk: nothing is
 * uploaded, the app reads them and decides what to send where.
 */

import { mkdir, readdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { asError } from "./error-utils";
import { hostCall } from "./host";
import { onHostEvent } from "./host-events";
import { tynd } from "./logger";
import { getWindow } from "./windows";

/** One crash report file. */
export interface CrashReport {
  /**
   * - `host-panic`: the native host crashed (`message`, `location`, `backtrace`);
   * - `backend-exit`: the Bun backend died (`exitCode`, last `stderr` lines);
   * - `render-process-gone`: a webview's renderer died (`label`, `reason`).
   */
  kind: "host-panic" | "backend-exit" | "render-process-gone";
  /** Milliseconds since the Unix epoch. */
  time: number;
  message: string;
  /** App version from `package.json` / `tynd.config.ts`, when known. */
  appVersion: string | null;
  tyndVersion?: string;
  /** `process.platform`-style OS name (`windows`, `macos`, `linux`) and CPU arch. */
  os: string;
  arch: string;
  location?: string | null;
  backtrace?: string;
  exitCode?: number | null;
  stderr?: string[];
  label?: string;
  reason?: RenderProcessGoneReason;
  /** Absolute path of the report file. */
  file: string;
}

/** Options of {@link crashReporter.start}. */
export interface CrashReporterOptions {
  /** Absolute directory for the reports. Default: `<local app data>/<app>/crashes`. */
  directory?: string;
  /**
   * Called once for every report this app has not seen yet: crashes of
   * earlier runs (on `start()`) and renderer losses of this run.
   */
  onCrash?: (report: CrashReport) => void;
}

/** Why a renderer process went away. */
export type RenderProcessGoneReason =
  | "crashed"
  | "unresponsive"
  | "browser-exited"
  | "frame-crashed";

/** Payload of `app.on("render-process-gone")`. */
export interface RenderProcessGoneEvent {
  /** Label of the webview whose renderer died. */
  label: string;
  reason: RenderProcessGoneReason;
  /** Reload the webview's window (`getWindow(label).reload()`). */
  reload(): Promise<void>;
}

const SEEN_FILE = ".last-seen";
const REPORT_FILE = /^(host-panic|backend-exit|render-process-gone)-\d+-\d+\.json$/;

let directory: string | undefined;
let onCrash: ((report: CrashReport) => void) | undefined;
let offProcessGone: (() => void) | undefined;

function requireDirectory(method: string): string {
  if (!directory) {
    throw new Error(`crashReporter.${method}() needs crashReporter.start() first`);
  }
  return directory;
}

async function readReport(dir: string, name: string): Promise<CrashReport | undefined> {
  try {
    const parsed = JSON.parse(await readFile(join(dir, name), "utf8")) as Partial<CrashReport>;
    if (typeof parsed.kind !== "string" || typeof parsed.time !== "number") return;
    return { ...parsed, file: join(dir, name) } as CrashReport;
  } catch {
    // A half-written or foreign file is not a report.
    return;
  }
}

async function listReports(dir: string): Promise<CrashReport[]> {
  const names = (await readdir(dir).catch(() => [] as string[])).filter((n) => REPORT_FILE.test(n));
  const reports = (await Promise.all(names.map((n) => readReport(dir, n)))).filter(
    (r): r is CrashReport => r !== undefined,
  );
  return reports.sort((a, b) => b.time - a.time);
}

async function lastSeen(dir: string): Promise<number> {
  const raw = await readFile(join(dir, SEEN_FILE), "utf8").catch(() => "0");
  const value = Number.parseInt(raw, 10);
  return Number.isFinite(value) ? value : 0;
}

function notify(report: CrashReport): void {
  try {
    onCrash?.(report);
  } catch (error) {
    tynd.error(`crashReporter onCrash threw: ${asError(error).message}`);
  }
}

/** Hand every not-yet-seen report to `onCrash`, oldest first, then remember the newest. */
async function deliverUnseen(dir: string): Promise<void> {
  const since = await lastSeen(dir);
  const unseen = (await listReports(dir)).filter((r) => r.time > since).reverse();
  for (const report of unseen) notify(report);
  const newest = unseen.at(-1)?.time;
  if (newest !== undefined) await writeFile(join(dir, SEEN_FILE), String(newest));
}

function appVersion(): string | null {
  const version = (globalThis as { __TYND_APP_INFO__?: { version?: unknown } }).__TYND_APP_INFO__
    ?.version;
  return typeof version === "string" && version ? version : null;
}

function osName(): string {
  if (process.platform === "win32") return "windows";
  if (process.platform === "darwin") return "macos";
  return process.platform;
}

async function recordProcessGone(dir: string, label: string, reason: RenderProcessGoneReason) {
  const time = Date.now();
  const name = `render-process-gone-${time}-${process.pid}.json`;
  const report: Omit<CrashReport, "file"> = {
    kind: "render-process-gone",
    time,
    message: `renderer of webview '${label}' is gone (${reason})`,
    label,
    reason,
    appVersion: appVersion(),
    os: osName(),
    arch: process.arch,
  };
  await writeFile(join(dir, name), JSON.stringify(report, null, 2));
  notify({ ...report, file: join(dir, name) });
  await writeFile(join(dir, SEEN_FILE), String(time));
}

function parseProcessGone(
  data: unknown,
): { label: string; reason: RenderProcessGoneReason } | null {
  const { label, reason } = (data ?? {}) as { label?: unknown; reason?: unknown };
  if (typeof label !== "string" || typeof reason !== "string") return null;
  return { label, reason: reason as RenderProcessGoneReason };
}

/**
 * Fires when a webview's renderer process dies (Windows: WebView2
 * `ProcessFailed`; macOS: web content process termination, after which the
 * host reloads the page itself; not reported on Linux). Returns the
 * unsubscribe function.
 */
export function onRenderProcessGone(handler: (event: RenderProcessGoneEvent) => void): () => void {
  return onHostEvent("webview:process-gone", (data) => {
    const parsed = parseProcessGone(data);
    if (!parsed) return;
    handler({ ...parsed, reload: () => getWindow(parsed.label).reload() });
  });
}

export const crashReporter = {
  /**
   * Start writing crash reports to `directory`. Call it after `app.start()`.
   * Resolves with the directory in use, after `onCrash` saw the reports of
   * earlier runs.
   * @example
   * app.start({ window: { title: "My App" } })
   * await app.crashReporter.start({
   *   onCrash: (report) => telemetry.send({ kind: report.kind, message: report.message }),
   * })
   */
  async start(options: CrashReporterOptions = {}): Promise<{ directory: string }> {
    if (options.directory !== undefined && typeof options.directory !== "string") {
      throw new TypeError("crashReporter.start: directory must be an absolute path string");
    }
    if (options.onCrash !== undefined && typeof options.onCrash !== "function") {
      throw new TypeError("crashReporter.start: onCrash must be a function");
    }
    const answer = await hostCall<{ directory: string }>("crashReporter", "start", {
      directory: options.directory ?? null,
      appVersion: appVersion(),
    });
    directory = answer.directory;
    onCrash = options.onCrash;
    await mkdir(directory, { recursive: true });
    offProcessGone?.();
    const dir = directory;
    offProcessGone = onHostEvent("webview:process-gone", (data) => {
      const parsed = parseProcessGone(data);
      if (!parsed) return;
      recordProcessGone(dir, parsed.label, parsed.reason).catch((error: unknown) =>
        tynd.error(`crash report not written: ${asError(error).message}`),
      );
    });
    await deliverUnseen(dir);
    return { directory: dir };
  },

  /** Every report in the directory, newest first. */
  getCrashReports(): Promise<CrashReport[]> {
    return listReports(requireDirectory("getCrashReports"));
  },

  /** The newest report, or `undefined` when there is none. */
  async getLastCrash(): Promise<CrashReport | undefined> {
    return (await listReports(requireDirectory("getLastCrash")))[0];
  },
};

/** @internal Forget the started state (tests). */
export function resetCrashReporterForTests(): void {
  offProcessGone?.();
  offProcessGone = undefined;
  directory = undefined;
  onCrash = undefined;
}
