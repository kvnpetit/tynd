/**
 * `@tynd/plugin-terminal` — single-import entry. PTY session handle
 * with bound `write` / `resize` / `kill` / `onData` / `onExit`.
 */

import { base64ToBytes, bytesToBase64 } from "@tynd/core/client/bridge";
import {
  composePluginClient,
  createResourceCloser,
  defineTypedPlugin,
  onHostEvent,
} from "@tynd/plugin-sdk";
import type { CommandsCatalog, SpawnArgs } from "./terminal.gen";

const _api = defineTypedPlugin<CommandsCatalog>("terminal");

/** Spawn configuration for `terminal.spawn`. */
export interface TerminalSpawnOptions {
  shell?: string;
  args?: string[];
  cwd?: string;
  env?: Record<string, string>;
  cols?: number;
  rows?: number;
}

function toSpawnArgs(opts: TerminalSpawnOptions): SpawnArgs {
  return {
    shell: opts.shell ?? null,
    args: opts.args ?? null,
    cwd: opts.cwd ?? null,
    env: opts.env ?? null,
    cols: opts.cols ?? null,
    rows: opts.rows ?? null,
  };
}

/** Active PTY session handle. */
export interface TerminalHandle {
  id: number;
  /** Write input bytes (or a string, encoded as UTF-8) to the PTY. */
  write(data: string | Uint8Array): Promise<void>;
  /** Resize the PTY window. */
  resize(cols: number, rows: number): Promise<void>;
  /** Kill the child process and detach all event listeners. */
  kill(): Promise<void>;
  /** Subscribe to PTY output bytes; returns an unsubscribe function. */
  onData(handler: (chunk: Uint8Array) => void): () => void;
  /** Subscribe to the child's exit code; returns an unsubscribe function. */
  onExit(handler: (code: number | null) => void): () => void;
}

/** Max PTY output bytes buffered while no `onData` handler is attached. */
const MAX_BACKLOG_BYTES = 1024 * 1024;

interface HostDataEvent {
  id?: unknown;
  data?: unknown;
}

interface HostExitEvent {
  id?: unknown;
  code?: unknown;
}

class TerminalEventHub {
  private readonly dataHandlers = new Set<(chunk: Uint8Array) => void>();
  private readonly exitHandlers = new Set<(code: number | null) => void>();
  private readonly dataBacklog: Uint8Array[] = [];
  private backlogBytes = 0;
  private exitCode: number | null = null;
  private exitSeen = false;
  private detached = false;
  private dataSubscriptionDisposed = false;
  private exitSubscriptionDisposed = false;
  private offData: () => void = () => undefined;
  private offExit: () => void = () => undefined;

  private readonly id: number;

  constructor(id: number) {
    this.id = id;
    try {
      this.offData = onHostEvent<HostDataEvent>("terminal:data", (raw) => this.handleHostData(raw));
      this.offExit = onHostEvent<HostExitEvent>("terminal:exit", (raw) => this.handleHostExit(raw));
    } catch (error) {
      try {
        this.detachHost();
      } catch {
        // Preserve the subscription error that prevented activation.
      }
      throw error;
    }
  }

  onData(handler: (chunk: Uint8Array) => void): () => void {
    if (this.detached) {
      // Output that arrived before the exit/kill is still owed to the first
      // listener; flush it, then there is nothing left to subscribe to.
      this.flushBacklog(handler);
      return () => undefined;
    }
    this.dataHandlers.add(handler);
    this.flushBacklog(handler);
    return this.removeFrom(this.dataHandlers, handler);
  }

  onExit(handler: (code: number | null) => void): () => void {
    if (this.exitSeen) {
      let active = true;
      queueMicrotask(() => {
        if (active) handler(this.exitCode);
      });
      return () => {
        active = false;
      };
    }
    if (this.detached) return () => undefined;
    this.exitHandlers.add(handler);
    return this.removeFrom(this.exitHandlers, handler);
  }

  private handleHostData(raw: HostDataEvent | undefined): void {
    if (this.detached || raw?.id !== this.id || typeof raw.data !== "string") return;
    const chunk = base64ToBytes(raw.data);
    if (this.dataHandlers.size === 0) {
      // Per-session backlog closes the brief activate-to-onData gap. It is
      // never global; native MAX_SESSIONS still bounds session count.
      this.dataBacklog.push(chunk);
      this.backlogBytes += chunk.byteLength;
      // Cap memory while nobody listens: drop the oldest chunks first.
      while (this.backlogBytes > MAX_BACKLOG_BYTES && this.dataBacklog.length > 1) {
        this.backlogBytes -= this.dataBacklog.shift()?.byteLength ?? 0;
      }
      return;
    }
    for (const handler of [...this.dataHandlers]) handler(chunk);
  }

  private handleHostExit(raw: HostExitEvent | undefined): void {
    if (this.detached || raw?.id !== this.id || this.exitSeen) return;
    this.exitSeen = true;
    this.exitCode = typeof raw.code === "number" ? raw.code : null;
    const handlers = [...this.exitHandlers];
    this.exitHandlers.clear();
    this.dataHandlers.clear();
    try {
      this.detachHost();
    } catch {
      // The process has exited; keep delivering the terminal state even
      // if a host unsubscribe implementation fails.
    }
    for (const handler of handlers) handler(this.exitCode);
  }

  private flushBacklog(handler: (chunk: Uint8Array) => void): void {
    if (this.dataBacklog.length === 0) return;
    const backlog = this.dataBacklog.splice(0);
    this.backlogBytes = 0;
    for (const chunk of backlog) handler(chunk);
  }

  dispose(): void {
    this.dataBacklog.length = 0;
    this.backlogBytes = 0;
    this.dataHandlers.clear();
    this.exitHandlers.clear();
    this.detachHost();
  }

  private removeFrom<T>(handlers: Set<T>, handler: T): () => void {
    let active = true;
    return () => {
      if (!active) return;
      active = false;
      handlers.delete(handler);
    };
  }

  private detachHost(): void {
    this.detached = true;
    let firstFailure: { error: unknown } | null = null;
    if (!this.dataSubscriptionDisposed) {
      try {
        this.offData();
        this.dataSubscriptionDisposed = true;
      } catch (error) {
        firstFailure = { error };
      }
    }
    if (!this.exitSubscriptionDisposed) {
      try {
        this.offExit();
        this.exitSubscriptionDisposed = true;
      } catch (error) {
        firstFailure ??= { error };
      }
    }
    if (firstFailure) throw firstFailure.error;
  }
}

function makeHandle(id: number): TerminalHandle {
  // Native reader/waiter remain gated until both subscriptions exist.
  const events = new TerminalEventHub(id);
  const kill = createResourceCloser(
    async () => {
      await _api.kill({ id });
    },
    () => events.dispose(),
  );
  return {
    id,
    async write(data) {
      const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data;
      await _api.write({ id, data: bytesToBase64(bytes) });
    },
    async resize(cols, rows) {
      await _api.resize({ id, cols, rows });
    },
    kill,
    onData(handler) {
      return events.onData(handler);
    },
    onExit(handler) {
      return events.onExit(handler);
    },
  };
}

/** Frontend bindings for the `@tynd/plugin-terminal` plugin (PTY sessions). */
export const terminal = composePluginClient(_api, {
  /** Spawn a PTY session and return a handle bound to its id. */
  async spawn(opts: TerminalSpawnOptions = {}): Promise<TerminalHandle> {
    const { id } = (await _api.spawn(toSpawnArgs(opts))) as { id: number };
    let handle: TerminalHandle | undefined;
    try {
      handle = makeHandle(id);
      await _api.activate({ id });
      return handle;
    } catch (error) {
      if (handle) await handle.kill().catch(() => undefined);
      else await _api.kill({ id }).catch(() => undefined);
      throw error;
    }
  },
  /** List ids of every still-running PTY session in this app. */
  list(): Promise<number[]> {
    return _api.list() as Promise<unknown> as Promise<number[]>;
  },
});

export default terminal;
