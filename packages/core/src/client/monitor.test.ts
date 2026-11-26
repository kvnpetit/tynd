import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type MockOsCall, type TyndMock } from "../testing";
import { type MonitorChange, monitors } from "./monitor";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

const osCalls = () => bridge.calls.filter((call): call is MockOsCall => call.kind === "os");

const display = {
  id: "DISPLAY1",
  name: "Dell U2720Q",
  position: { x: 0, y: 0 },
  size: { width: 1920, height: 1080 },
  workArea: { x: 0, y: 0, width: 1920, height: 1032 },
  scale: 1,
  isPrimary: true,
};

describe("monitors", () => {
  test("all() returns the host list with work areas", async () => {
    bridge.setOs("window", "monitors", () => [display]);
    const list = await monitors.all();
    expect(list[0]?.workArea.height).toBe(1032);
  });

  test("onChange subscribes once and delivers monitor:change until disposed", async () => {
    const seen: MonitorChange[] = [];
    const first = await monitors.onChange((change) => seen.push(change));
    const second = await monitors.onChange(() => undefined);
    const change = { added: [], removed: [], changed: [display], monitors: [display] };
    bridge.emitOs("monitor:change", change);
    first.dispose();
    bridge.emitOs("monitor:change", change);
    second.dispose();
    expect(seen).toEqual([change]);
    expect(osCalls().filter((c) => c.method === "subscribeMonitorChanges")).toHaveLength(1);
  });
});
