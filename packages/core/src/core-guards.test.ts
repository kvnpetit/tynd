import { afterEach, describe, expect, test } from "bun:test";
import { fileURLToPath } from "node:url";
import * as v from "valibot";
import { hostCall, resolveHostCall, setHostCallTransport } from "./host";
import { app } from "./index";
import { fireClose, fireReady, lifecycleFlags, onCloseFns, onReadyFns } from "./lifecycle";
import { definePlugin } from "./plugin";
import { getPluginMethod, registerPlugin, resetPluginRegistry } from "./runtime/plugin-registry";
import { type AppConfig, AppConfigSchema } from "./types";

describe("plugin registry", () => {
  afterEach(() => resetPluginRegistry());

  test("methods keep their `this` binding", async () => {
    registerPlugin(
      definePlugin({
        name: "bound",
        methods: {
          a(): number {
            return (this as unknown as { b(): number }).b() + 1;
          },
          b(): number {
            return 41;
          },
        },
      }),
    );
    const a = getPluginMethod("bound", "a");
    expect(await a?.()).toBe(42);
  });

  test("registering the same plugin twice is a no-op", () => {
    const plugin = definePlugin({ name: "twice", methods: { ping: () => "pong" } });
    registerPlugin(plugin);
    expect(() => registerPlugin(plugin)).not.toThrow();
  });

  test("a different plugin under a taken name throws", () => {
    registerPlugin(definePlugin({ name: "dup", methods: { one: () => 1 } }));
    expect(() => registerPlugin(definePlugin({ name: "dup", methods: { two: () => 2 } }))).toThrow(
      /already registered/,
    );
    expect(getPluginMethod("dup", "one")).not.toBeNull();
    expect(getPluginMethod("dup", "two")).toBeNull();
  });
});

describe("hostCall deadline", () => {
  afterEach(() => setHostCallTransport(null));

  test("a lost host_result rejects after timeoutMs", async () => {
    setHostCallTransport(() => undefined);
    await expect(hostCall("security", "checkFs", null, { timeoutMs: 15 })).rejects.toThrow(
      /timed out after 15ms/,
    );
  });

  test("a prompt answer clears the timer and resolves", async () => {
    let id = "";
    setHostCallTransport((frame) => {
      id = frame.id;
    });
    const p = hostCall<string>("security", "checkFs", null, { timeoutMs: 15 });
    resolveHostCall(id, true, "ok");
    expect(await p).toBe("ok");
    await Bun.sleep(30);
  });
});

describe("lifecycle handlers", () => {
  afterEach(() => {
    lifecycleFlags.readyFired = false;
    lifecycleFlags.closeFired = false;
  });

  test("a throwing or rejecting handler does not block the others nor go unhandled", async () => {
    const unhandled: unknown[] = [];
    const onUnhandled = (e: unknown) => unhandled.push(e);
    process.on("unhandledRejection", onUnhandled);
    try {
      const ran: string[] = [];
      onReadyFns.push(() => {
        throw new Error("sync");
      });
      onReadyFns.push((async () => {
        throw new Error("async");
      }) as () => void);
      onReadyFns.push(() => {
        ran.push("ready");
      });
      onCloseFns.push((async () => {
        throw new Error("async close");
      }) as () => void);
      onCloseFns.push(() => {
        ran.push("close");
      });
      await fireReady();
      fireClose();
      await Bun.sleep(20);
      expect(ran).toEqual(["ready", "close"]);
      expect(unhandled).toEqual([]);
    } finally {
      process.off("unhandledRejection", onUnhandled);
    }
  });
});

describe("app.start", () => {
  test("rejects an unknown option key, naming it and listing the valid ones", () => {
    let message = "";
    try {
      app.start({ plugins: [] } as unknown as AppConfig);
    } catch (e) {
      message = (e as Error).message;
    }
    expect(message).toContain('plugins: unknown option "plugins"');
    expect(message).toContain("valid options: window, menu, tray, security, capabilities");
    expect(message).toContain("quitOnLastWindowClosed");
  });

  test("a second call throws instead of attaching a second listener", async () => {
    const entry = fileURLToPath(new URL("./runtime/bun.fixture.ts", import.meta.url));
    const index = fileURLToPath(new URL("./index.ts", import.meta.url));
    const script = `
      import { app } from ${JSON.stringify(index)};
      app.start({});
      try { app.start({}); console.log("NO_THROW"); } catch (e) { console.log("THROWS:" + e.message); }
      process.exit(0);
    `;
    const proc = Bun.spawn([process.execPath, "-e", script], {
      env: { ...process.env, TYND_ENTRY: entry },
      stdout: "ignore",
      stderr: "pipe",
      stdin: "ignore",
    });
    const out = await new Response(proc.stderr).text();
    await proc.exited;
    expect(out).toContain("THROWS:app.start() was already called");
    expect(out).not.toContain("NO_THROW");
  });
});

describe("app.start nested options", () => {
  test.each([
    [
      { window: { titel: "x" } },
      'window.titel: unknown option "titel"; valid options: title, width',
    ],
    [{ security: { cspp: "x" } }, 'security.cspp: unknown option "cspp"; valid options: fs, http'],
    [
      { security: { fs: { alow: [] } } },
      'security.fs.alow: unknown option "alow"; valid options: allow, deny',
    ],
    [{ window: { gpu: { vendor: "amd", id: 1 } } }, 'window.gpu.id: unknown option "id"'],
    [{ tray: { icon: "i.png", tooltp: "x" } }, 'tray.tooltp: unknown option "tooltp"'],
    [
      { menu: [{ type: "submenu", label: "File", items: [{ label: "Open", checked: true }] }] },
      'menu.0.items.0.checked: unknown option "checked"',
    ],
    [
      { menu: [{ type: "submenu", label: "File", items: [{ type: "chekbox", label: "x" }] }] },
      'menu.0.items.0.type: Invalid type: Expected ("separator" | "item"',
    ],
    [
      { capabilities: [{ identifier: "main", remote: { url: [] } }] },
      'capabilities.0.remote.url: unknown option "url"; valid options: urls',
    ],
    [
      { capabilities: [{ identifier: "main", permissions: [{ identifier: "fs", comands: {} }] }] },
      'capabilities.0.permissions.0.comands: unknown option "comands"',
    ],
  ])("rejects an unknown nested key before starting: %j", (config, expected) => {
    expect(() => app.start(config as unknown as AppConfig)).toThrow(expected);
  });

  test("open records stay open", () => {
    const config = {
      capabilities: [
        { identifier: "main", permissions: [{ identifier: "fs", scope: { any: 1 } }] },
      ],
    };
    expect(v.safeParse(AppConfigSchema, config).success).toBe(true);
  });
});
