import { dlopen, FFIType, ptr } from "bun:ffi";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  DebugClient,
  type DebugClientOptions,
  type ScreenshotResult,
  type Transport,
} from "./client.ts";
import {
  type DiscoveryEntry,
  defaultDiscoveryDir,
  isProcessAlive,
  readDiscoveryEntries,
  selectEntry,
} from "./discovery.ts";
import {
  type ExpectScreenshotOptions,
  type ExpectScreenshotResult,
  expectScreenshot,
} from "./screenshot.ts";
import { connectTransport } from "./transport.ts";
import { type WaitForOptions, waitFor } from "./wait.ts";

export interface LaunchOptions extends DebugClientOptions {
  /** Project root. Defaults to `TYND_TEST_PROJECT` (set by `tynd test`), then the current directory. */
  cwd?: string;
  /** Extra arguments appended to the launch command (`tynd dev ...` by default). */
  args?: string[];
  /** Extra environment variables for the app process. */
  env?: Record<string, string | undefined>;
  /**
   * Command that starts the app. Defaults to `[TYND_TEST_CLI ?? "tynd", "dev"]`. The command must
   * end up running a Tynd host with the debug socket enabled.
   */
  command?: string[];
  /**
   * Milliseconds to wait for the app to expose its debug socket and become ready. Defaults to
   * 120000, lowered so it stays below the per-test timeout `tynd test` exports in
   * `TYND_TEST_TIMEOUT_MS` (a launch that outlives its test would leak the app).
   */
  timeout?: number;
  /** Only attach to a host reporting this app id (useful when several apps run). */
  appId?: string;
  /** Wait until a window exists and its document is loaded (default true). */
  waitForReady?: boolean;
  /** Where the host publishes discovery files; defaults to the platform location. */
  discoveryDir?: string;
}

const DEFAULT_LAUNCH_TIMEOUT_MS = 120_000;
/** Time reserved after a failed launch to kill the process tree before the test itself times out. */
const CLEANUP_MARGIN_MS = 10_000;
const OUTPUT_LIMIT = 64 * 1024;

/** Default launch timeout: 120 s, kept below the per-test timeout `tynd test` hands to bun. */
export function defaultLaunchTimeout(
  env: Record<string, string | undefined> = process.env,
): number {
  const testTimeout = Number(env.TYND_TEST_TIMEOUT_MS);
  if (!Number.isFinite(testTimeout) || testTimeout <= 0) return DEFAULT_LAUNCH_TIMEOUT_MS;
  return Math.min(DEFAULT_LAUNCH_TIMEOUT_MS, Math.max(1_000, testTimeout - CLEANUP_MARGIN_MS));
}

type Child = ReturnType<typeof Bun.spawn>;

interface AppParts {
  transport: Transport;
  child: Child;
  entry: DiscoveryEntry;
  output: { text: string };
  /** Private temp directory holding the debug socket (Unix); removed on close. */
  socketDir?: string;
}

/** A running Tynd app: the debug client plus the process it controls. */
export class TyndApp extends DebugClient {
  readonly pid: number;
  readonly entry: DiscoveryEntry;
  readonly #child: Child;
  readonly #output: { text: string };
  readonly #socketDir: string | undefined;
  #exited = false;
  #closing: Promise<void> | undefined;

  constructor(parts: AppParts, options: DebugClientOptions) {
    super(parts.transport, options);
    this.#child = parts.child;
    this.pid = parts.entry.pid;
    this.entry = parts.entry;
    this.#output = parts.output;
    this.#socketDir = parts.socketDir;
    void parts.child.exited.then(() => {
      this.#exited = true;
    });
  }

  /** Last stdout/stderr of the launch command, for diagnostics. */
  output(): string {
    return this.#output.text;
  }

  /** Poll until `predicate` is truthy; see {@link waitFor}. */
  waitFor<T>(predicate: () => T | Promise<T>, options?: WaitForOptions) {
    return waitFor(predicate, options);
  }

  /** Poll until `selector` matches an element and resolve with it. */
  waitForSelector(selector: string, options?: WaitForOptions & { label?: string }) {
    return waitFor(() => this.findElement(selector, { label: options?.label }), {
      message: `element ${selector}`,
      ...options,
    });
  }

  /** Take a screenshot and compare it with the baseline file (created when missing). */
  async expectScreenshot(
    baselinePath: string,
    options: ExpectScreenshotOptions & { label?: string } = {},
  ): Promise<ExpectScreenshotResult> {
    const shot: ScreenshotResult = await this.screenshot({ label: options.label });
    return expectScreenshot(shot.png, baselinePath, options);
  }

  /** Ask the app to quit, then make sure the process tree is gone. Safe to call twice. */
  override close(): Promise<void> {
    this.#closing ??= this.#shutdown();
    return this.#closing;
  }

  async #shutdown(): Promise<void> {
    // Windows cannot find a tree once its leader exited: record it while it is alive.
    const tree = windowsTreePids([this.#child.pid, this.pid]);
    if (!this.closed) {
      await Promise.race([this.quit().catch(() => false), sleep(2_000)]);
    }
    super.close();
    await Promise.race([this.#child.exited, sleep(3_000)]);
    // Always sweep: the launcher (`tynd dev`) can exit on quit while the dev server and host linger.
    await killTree(this.#child, tree);
    removeDir(this.#socketDir);
  }
}

/** Start the app, wait for its debug socket, authenticate and (by default) wait for a ready window. */
export async function launchApp(options: LaunchOptions = {}): Promise<TyndApp> {
  const cwd = options.cwd ?? process.env.TYND_TEST_PROJECT ?? process.cwd();
  const command = options.command ?? [resolveCli(cwd), "dev"];
  const program = command[0];
  if (!program) throw new Error("launchApp: empty command");
  const timeout = options.timeout ?? defaultLaunchTimeout();
  const discoveryDir = options.discoveryDir ?? defaultDiscoveryDir();
  const requestedSocket = options.env?.TYND_DEBUG_SOCKET;
  const socketDir = requestedSocket ? undefined : makeSocketDir();
  const socket = requestedSocket || socketPath(socketDir);
  const startedAt = Date.now();

  const output = { text: "" };
  const child = Bun.spawn(
    [Bun.which(program) ?? program, ...command.slice(1), ...(options.args ?? [])],
    {
      cwd,
      env: { ...process.env, ...options.env, TYND_DEBUG_SOCKET: socket },
      stdin: "ignore",
      stdout: "pipe",
      stderr: "pipe",
      // Own process group on Unix so the whole tree (host, Bun, dev server) can be signalled at once.
      detached: process.platform !== "win32",
    },
  );
  trackChild(child);
  void collect(child.stdout, output);
  void collect(child.stderr, output);

  try {
    const entry = await discover(child, {
      dir: discoveryDir,
      notBefore: startedAt - 1_000,
      socket,
      appId: options.appId,
      deadline: startedAt + timeout,
    });
    return await connectApp({ child, entry, output, socketDir }, options, startedAt + timeout);
  } catch (error) {
    await killTree(child, windowsTreePids([child.pid]));
    removeDir(socketDir);
    const reason = error instanceof Error ? error.message : String(error);
    throw new Error(
      `launchApp failed: ${reason}\n--- app output ---\n${output.text.slice(-4_000)}`,
    );
  }
}

async function connectApp(
  launched: Omit<AppParts, "transport">,
  options: LaunchOptions,
  deadline: number,
): Promise<TyndApp> {
  const transport = await connectTransport(
    launched.entry.endpoint,
    Math.max(1_000, deadline - Date.now()),
  );
  const app = new TyndApp({ ...launched, transport }, options);
  try {
    await app.hello(launched.entry.token);
    if (options.waitForReady ?? true) {
      await app.waitFor(
        async () => {
          const windows = await app.windowList();
          if (windows.length === 0) return false;
          return (await app.eval<string>("document.readyState")) === "complete";
        },
        { timeout: Math.max(1_000, deadline - Date.now()), message: "app window ready" },
      );
    }
  } catch (error) {
    await app.close();
    throw error;
  }
  return app;
}

/** Resolve the `tynd` executable: `TYND_TEST_CLI`, the project's local install, then `PATH`. */
export function resolveCli(cwd: string): string {
  const fromEnv = process.env.TYND_TEST_CLI;
  if (fromEnv) return fromEnv;
  const bin = join(cwd, "node_modules", ".bin");
  for (const name of process.platform === "win32" ? ["tynd.exe", "tynd.cmd"] : ["tynd"]) {
    const candidate = join(bin, name);
    if (existsSync(candidate)) return candidate;
  }
  return "tynd";
}

/** Fresh private (mode 0700) directory for the debug socket; none on Windows (named pipe). */
function makeSocketDir(): string | undefined {
  if (process.platform === "win32") return;
  return mkdtempSync(join(tmpdir(), "tynd-test-"));
}

function socketPath(dir: string | undefined): string {
  if (dir === undefined) {
    const id = `${process.pid}-${Math.random().toString(36).slice(2, 8)}`;
    return `\\\\.\\pipe\\tynd-test-${id}`;
  }
  return join(dir, "d.sock");
}

function removeDir(dir: string | undefined): void {
  if (dir === undefined) return;
  try {
    rmSync(dir, { recursive: true, force: true });
  } catch {
    // Best effort: a leftover private temp directory is harmless.
  }
}

async function discover(
  child: Child,
  options: {
    dir: string;
    notBefore: number;
    socket: string;
    appId?: string;
    deadline: number;
  },
): Promise<DiscoveryEntry> {
  let exitCode: number | undefined;
  void child.exited.then((code) => {
    exitCode = code;
  });
  for (;;) {
    const entry = selectEntry(readDiscoveryEntries(options.dir), {
      notBefore: options.notBefore,
      socket: options.socket,
      launcherPid: child.pid,
      appId: options.appId,
    });
    if (entry) return entry;
    if (exitCode !== undefined) {
      throw new Error(`the app exited with code ${exitCode} before exposing a debug socket`);
    }
    if (Date.now() >= options.deadline) {
      throw new Error(
        `no debug socket appeared in ${options.dir} (is the debug protocol enabled?)`,
      );
    }
    await sleep(100);
  }
}

async function collect(
  stream: ReadableStream<Uint8Array> | number | undefined | null,
  output: { text: string },
): Promise<void> {
  if (!stream || typeof stream === "number") return;
  const decoder = new TextDecoder();
  try {
    for await (const chunk of stream) {
      output.text = (output.text + decoder.decode(chunk, { stream: true })).slice(-OUTPUT_LIMIT);
    }
  } catch {
    // Stream closed with the process.
  }
}

/**
 * Kill the launched app's whole tree, also when the leader already exited.
 * POSIX: the process group is always signalled. Windows: `taskkill /T` needs a live
 * leader, so the descendants recorded earlier by {@link windowsTreePids} are killed too.
 */
async function killTree(child: Child, knownPids: number[] = []): Promise<void> {
  if (process.platform === "win32") {
    if (child.exitCode === null) killTreeSync(child);
    killPidsSync(knownPids);
  } else {
    if (child.exitCode === null) {
      signalGroup(child, "SIGTERM");
      await Promise.race([child.exited, sleep(2_000)]);
    }
    // The leader may be gone while its children linger: always finish the group off.
    signalGroup(child, "SIGKILL");
  }
  await Promise.race([child.exited, sleep(2_000)]);
}

/** Pids reachable from `roots` through (pid, parent pid) edges, roots included. */
export function descendantPids(roots: number[], edges: ReadonlyArray<[number, number]>): number[] {
  const found = new Set(roots.filter((pid) => Number.isInteger(pid) && pid > 0));
  for (let grew = true; grew; ) {
    grew = false;
    for (const [pid, parent] of edges) {
      if (found.has(parent) && !found.has(pid)) {
        found.add(pid);
        grew = true;
      }
    }
  }
  return [...found];
}

/**
 * Every (pid, parent pid) pair of the running processes, from a Toolhelp snapshot via
 * `kernel32` (about 40 ms; spawning PowerShell for this costs seconds).
 */
function windowsProcessEdges(): Array<[number, number]> {
  const kernel = dlopen("kernel32.dll", {
    CreateToolhelp32Snapshot: { args: [FFIType.u32, FFIType.u32], returns: FFIType.ptr },
    Process32FirstW: { args: [FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
    Process32NextW: { args: [FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
    CloseHandle: { args: [FFIType.ptr], returns: FFIType.i32 },
  });
  const TH32CS_SNAPPROCESS = 0x2;
  // PROCESSENTRY32W (64-bit layout): dwSize at 0, th32ProcessID at 8, th32ParentProcessID at 32.
  const PROCESSENTRY32W_SIZE = 568;
  const snapshot = kernel.symbols.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
  if (!snapshot) return [];
  try {
    const entry = new Uint8Array(PROCESSENTRY32W_SIZE);
    const view = new DataView(entry.buffer);
    view.setUint32(0, PROCESSENTRY32W_SIZE, true);
    const edges: Array<[number, number]> = [];
    for (
      let ok = kernel.symbols.Process32FirstW(snapshot, ptr(entry));
      ok;
      ok = kernel.symbols.Process32NextW(snapshot, ptr(entry))
    ) {
      edges.push([view.getUint32(8, true), view.getUint32(32, true)]);
    }
    return edges;
  } finally {
    kernel.symbols.CloseHandle(snapshot);
  }
}

/** Windows only: snapshot `roots` and every descendant. Returns [] elsewhere or on failure. */
function windowsTreePids(roots: number[]): number[] {
  if (process.platform !== "win32") return [];
  try {
    return descendantPids(roots, windowsProcessEdges());
  } catch {
    return roots.filter((pid) => Number.isInteger(pid) && pid > 0);
  }
}

/** The subset of `pids` still running (`taskkill` costs ~0.7 s per spawn under load, even for dead pids). */
export function livePids(
  pids: number[],
  alive: (pid: number) => boolean = isProcessAlive,
): number[] {
  return pids.filter((pid) => Number.isInteger(pid) && pid > 0 && alive(pid));
}

function killPidsSync(pids: number[]): void {
  const live = livePids(pids);
  if (live.length === 0) return;
  Bun.spawnSync(["taskkill", ...live.flatMap((pid) => ["/pid", String(pid)]), "/T", "/F"], {
    stdout: "ignore",
    stderr: "ignore",
  });
}

/** Signal the child's whole process group (the child is its leader, see `detached`). */
function signalGroup(child: Child, signal: NodeJS.Signals): void {
  try {
    process.kill(-child.pid, signal);
  } catch {
    try {
      child.kill(signal);
    } catch {
      // Already gone.
    }
  }
}

/** Synchronous hard kill of the whole tree, usable from `process.on("exit")`. */
function killTreeSync(child: Child): void {
  if (process.platform === "win32") {
    Bun.spawnSync(["taskkill", "/pid", String(child.pid), "/T", "/F"], {
      stdout: "ignore",
      stderr: "ignore",
    });
  } else {
    signalGroup(child, "SIGKILL");
  }
}

const liveChildren = new Set<Child>();
let exitHooksInstalled = false;

/** Remember a launched app so it is killed when the test process exits or is interrupted. */
function trackChild(child: Child): void {
  liveChildren.add(child);
  void child.exited.then(() => liveChildren.delete(child));
  if (exitHooksInstalled) return;
  exitHooksInstalled = true;
  const killAll = () => {
    for (const live of liveChildren) killTreeSync(live);
    liveChildren.clear();
  };
  process.on("exit", killAll);
  for (const [signal, code] of [
    ["SIGINT", 130],
    ["SIGTERM", 143],
  ] as const) {
    process.on(signal, () => {
      killAll();
      process.exit(code);
    });
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
