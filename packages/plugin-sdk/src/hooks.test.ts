import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { registerPlugin } from "@tynd/core";
import { fireHook } from "../../core/src/runtime/hooks";
import { clearMiddleware, runMiddleware } from "../../core/src/runtime/middleware";
import { resetPluginRegistry } from "../../core/src/runtime/plugin-registry";
import { defineTyndPlugin } from "./hooks";

const noop = () => undefined;

beforeEach(() => {
  resetPluginRegistry();
  clearMiddleware();
});
afterEach(() => {
  resetPluginRegistry();
  clearMiddleware();
});

describe("defineTyndPlugin + registerPlugin", () => {
  test("every runtime hook fires on a real hookable instance", async () => {
    const seen: string[] = [];
    const plugin = defineTyndPlugin({
      name: "demo",
      hooks: {
        "app:beforeStart": (cx, config) => {
          seen.push(`beforeStart:${cx.name}:${typeof config}`);
        },
        "app:ready": () => {
          seen.push("ready");
        },
        "app:beforeQuit": () => {
          seen.push("beforeQuit");
        },
        "window:created": (_cx, win) => {
          seen.push(`created:${win.label}`);
        },
        "window:closed": (_cx, win) => {
          seen.push(`closed:${win.label}`);
        },
        "ipc:before": (_cx, cmd) => {
          seen.push(`before:${cmd}`);
        },
        "ipc:after": (_cx, cmd, result) => {
          seen.push(`after:${cmd}:${String(result)}`);
        },
      },
    });
    registerPlugin(plugin);

    await fireHook("app:beforeStart", {});
    await fireHook("app:ready");
    await fireHook("window:created", { label: "w1" });
    await fireHook("window:closed", { label: "w1" });
    await runMiddleware({ fn: "ping", args: [], caller: undefined }, () => "pong");
    await fireHook("app:beforeQuit");

    expect(seen).toEqual([
      "beforeStart:demo:object",
      "ready",
      "created:w1",
      "closed:w1",
      "before:ping",
      "after:ping:pong",
      "beforeQuit",
    ]);
  });
});

describe("ipc rejection and validation", () => {
  test("ipc:before rejects a plugin method call", async () => {
    registerPlugin(
      defineTyndPlugin({
        name: "gate",
        hooks: {
          "ipc:before": (_cx, cmd) => {
            if (cmd === "secret") throw new Error("forbidden");
          },
        },
      }),
    );
    await expect(
      runMiddleware({ fn: "secret", args: [], caller: undefined }, () => "x"),
    ).rejects.toThrow("forbidden");
    expect(await runMiddleware({ fn: "open", args: [], caller: undefined }, () => "x")).toBe("x");
  });

  test("unknown hook names are still rejected", () => {
    expect(() => defineTyndPlugin({ name: "bad", hooks: { "app:nope": noop } as never })).toThrow(
      "unknown hook",
    );
  });

  test("removed build hooks are rejected as unknown", () => {
    for (const name of ["build:beforeBundle", "build:afterBundle"]) {
      expect(() => defineTyndPlugin({ name: "old", hooks: { [name]: noop } as never })).toThrow(
        `unknown hook '${name}'`,
      );
    }
  });
});
