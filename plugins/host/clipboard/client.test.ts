import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { clipboard } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-clipboard client", () => {
  test("writeText", async () => {
    await clipboard.writeText("hi");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "clipboard", method: "writeText", args: { text: "hi" } },
    ]);
  });

  test("writeHtml omits an absent alt", async () => {
    await clipboard.writeHtml("<b>x</b>");
    expect(bridge.calls).toEqual([
      { kind: "os", api: "clipboard", method: "writeHtml", args: { html: "<b>x</b>" } },
    ]);
  });
});

// Tests below drive the client through the full core mock (os_on events, host errors).
let mock: TyndMock | undefined;

describe("clipboard.onChange shared watcher", () => {
  afterEach(() => {
    mock?.restore();
    mock = undefined;
  });

  test("the host watcher stops only when the last subscriber leaves", async () => {
    const seen: string[] = [];
    mock = installTyndMock({
      os: {
        "clipboard.startMonitoring": () => {
          seen.push("start");
        },
        "clipboard.stopMonitoring": () => {
          seen.push("stop");
        },
      },
    });
    const received: string[] = [];
    const offA = await clipboard.onChange((e) => received.push(`a:${e.text}`));
    const offB = await clipboard.onChange((e) => received.push(`b:${e.text}`));
    expect(seen).toEqual(["start"]);

    await offA();
    await offA(); // idempotent
    expect(seen).toEqual(["start"]);
    mock.emitOs("clipboard:change", { text: "x" });
    expect(received).toEqual(["b:x"]);

    await offB();
    expect(seen).toEqual(["start", "stop"]);
  });
});
