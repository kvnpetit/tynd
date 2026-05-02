import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { process } from "./process";

type OsCall = { api: string; method: string; args: unknown };
type EventHandler = (data: unknown) => void;

interface Shim {
  osCalls: OsCall[];
  emit(name: string, data: unknown): void;
  listeners(name: string): EventHandler[];
}

function mountShim(spawnReturn: { pid: number } = { pid: 4242 }): Shim {
  const osCalls: OsCall[] = [];
  const eventListeners: Map<string, EventHandler[]> = new Map();

  (globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        osCalls.push({ api, method, args });
        if (api === "process" && method === "spawn") {
          return Promise.resolve(spawnReturn);
        }
        return Promise.resolve(null);
      },
      os_on(name: string, handler: EventHandler) {
        const arr = eventListeners.get(name) ?? [];
        arr.push(handler);
        eventListeners.set(name, arr);
        return () => {
          const list = eventListeners.get(name);
          if (!list) return;
          const idx = list.indexOf(handler);
          if (idx >= 0) list.splice(idx, 1);
        };
      },
      call() {
        return Promise.resolve(null);
      },
      on() {
        return () => undefined;
      },
      off() {
        return undefined;
      },
    },
  };

  return {
    osCalls,
    emit(name, data) {
      const list = eventListeners.get(name);
      if (!list) return;
      for (const h of list.slice()) h(data);
    },
    listeners(name) {
      return eventListeners.get(name) ?? [];
    },
  };
}

describe("process.spawn — wire format + streaming", () => {
  let shim: Shim;
  beforeEach(() => {
    shim = mountShim();
  });
  afterEach(() => {
    (globalThis as { window?: unknown }).window = undefined;
  });

  test("spawn forwards cmd + opts and returns a handle with the assigned pid", async () => {
    const proc = await process.spawn("git", {
      args: ["status"],
      cwd: "/tmp",
      env: { FOO: "bar" },
    });
    expect(shim.osCalls).toEqual([
      {
        api: "process",
        method: "spawn",
        args: { cmd: "git", args: ["status"], cwd: "/tmp", env: { FOO: "bar" } },
      },
    ]);
    expect(proc.pid).toBe(4242);
  });

  test("stdout iterable yields chunks emitted by the host", async () => {
    const proc = await process.spawn("echo", { args: ["hello"] });
    const collected: Uint8Array[] = [];

    // Drain stdout in the background.
    const drainer = (async () => {
      for await (const chunk of proc.stdout) collected.push(chunk);
    })();

    // Allow the iterator's async setup to register listeners.
    await Promise.resolve();
    await Promise.resolve();

    // Emit two chunks then exit — host wire format is `{ data: number[] }`.
    shim.emit("process:stdout:4242", { data: [104, 105] }); // "hi"
    shim.emit("process:stdout:4242", { data: [33] }); // "!"
    shim.emit("process:exit:4242", { code: 0, signal: null });

    await drainer;
    const merged = new Uint8Array(collected.reduce((acc, c) => acc + c.length, 0));
    let off = 0;
    for (const c of collected) {
      merged.set(c, off);
      off += c.length;
    }
    expect(new TextDecoder().decode(merged)).toBe("hi!");
  });

  test("exit() resolves with the host-provided code + signal", async () => {
    const proc = await process.spawn("sleep", { args: ["1"] });
    setTimeout(() => shim.emit("process:exit:4242", { code: 7, signal: null }), 0);
    const info = await proc.exit();
    expect(info).toEqual({ code: 7, signal: null });
  });

  test("kill forwards pid + signal to the host", async () => {
    const proc = await process.spawn("sleep", { args: ["10"] });
    await proc.kill("SIGKILL");
    expect(shim.osCalls.at(-1)).toEqual({
      api: "process",
      method: "kill",
      args: { pid: 4242, signal: "SIGKILL" },
    });
  });

  test("stdin.write encodes string as-is and Uint8Array as byte array", async () => {
    const proc = await process.spawn("cat");
    await proc.stdin.write("hello");
    await proc.stdin.write(new Uint8Array([1, 2, 3]));
    expect(shim.osCalls.slice(-2)).toEqual([
      { api: "process", method: "writeStdin", args: { pid: 4242, data: "hello" } },
      { api: "process", method: "writeStdin", args: { pid: 4242, data: [1, 2, 3] } },
    ]);
  });

  test("stdin.close issues a closeStdin call", async () => {
    const proc = await process.spawn("cat");
    await proc.stdin.close();
    expect(shim.osCalls.at(-1)).toEqual({
      api: "process",
      method: "closeStdin",
      args: { pid: 4242 },
    });
  });
});

describe("process.exec — kept stable across W7.3", () => {
  let shim: Shim;
  beforeEach(() => {
    shim = mountShim();
  });
  afterEach(() => {
    (globalThis as { window?: unknown }).window = undefined;
  });

  test("exec routes through process.exec with merged opts", async () => {
    await process.exec("ls", { args: ["-la"], cwd: "/tmp" });
    expect(shim.osCalls).toEqual([
      { api: "process", method: "exec", args: { cmd: "ls", args: ["-la"], cwd: "/tmp" } },
    ]);
  });
});
