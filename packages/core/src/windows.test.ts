import { afterEach, describe, expect, test } from "bun:test";
import { resolveHostCall, setHostCallTransport } from "./host";
import { dispatchHostEvent } from "./host-events";
import { getWindow } from "./windows";

interface Frame {
  id: string;
  api: string;
  method: string;
  args: unknown;
}

afterEach(() => setHostCallTransport(null));

function capture(): Frame[] {
  const frames: Frame[] = [];
  setHostCallTransport((frame) => {
    frames.push(frame);
    queueMicrotask(() => resolveHostCall(frame.id, true, null));
  });
  return frames;
}

describe("backend getWindow", () => {
  test("targets the primary window by default and a label when given", async () => {
    const frames = capture();
    await getWindow().setTitle("Ready");
    await getWindow("settings").show();
    expect(frames.map(({ api, method, args }) => ({ api, method, args }))).toEqual([
      { api: "window", method: "setTitle", args: { title: "Ready", window: "main" } },
      { api: "window", method: "show", args: { window: "settings" } },
    ]);
  });

  test("close sends the handle's label", async () => {
    const frames = capture();
    await getWindow("settings").close();
    expect(frames[0]?.args).toEqual({ label: "settings" });
  });

  test("events forwarded by the host reach the handle of that window only", () => {
    const seen: unknown[] = [];
    const off = getWindow("editor").onResized((e) => seen.push(e));
    dispatchHostEvent("window:resized", { label: "main", width: 1, height: 1 });
    dispatchHostEvent("window:resized", { label: "editor", width: 800, height: 600 });
    off();
    dispatchHostEvent("window:resized", { label: "editor", width: 1, height: 1 });
    expect(seen).toEqual([{ label: "editor", width: 800, height: 600 }]);
  });

  test("preventDefault on close-requested cancels that window's close", async () => {
    const frames = capture();
    const off = getWindow("editor").onCloseRequested((event) => event.preventDefault());
    dispatchHostEvent("window:close-requested", { label: "main" });
    dispatchHostEvent("window:close-requested", { label: "editor" });
    off();
    await Bun.sleep(1);
    expect(frames.map(({ method, args }) => ({ method, args }))).toEqual([
      { method: "cancelClose", args: { window: "editor" } },
    ]);
  });

  test("a throwing handler does not stop the others", () => {
    let reached = false;
    const offA = getWindow().onFocused(() => {
      throw new Error("boom");
    });
    const offB = getWindow().onFocused(() => {
      reached = true;
    });
    dispatchHostEvent("window:focused", { label: "main" });
    offA();
    offB();
    expect(reached).toBe(true);
  });
});
