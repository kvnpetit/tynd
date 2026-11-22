import { afterEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { power } from "./client";

let mock: TyndMock | undefined;
afterEach(() => {
  mock?.restore();
  mock = undefined;
});

describe("power", () => {
  test("getBattery maps to power.battery", async () => {
    const battery = { percent: 80, charging: true, secondsRemaining: 600, status: "charging" };
    mock = installTyndMock({ os: { "power.battery": () => battery } });
    expect(await power.getBattery()).toEqual(battery as never);
  });

  test("an event subscription rejects when the host watcher cannot start, then retries", async () => {
    let attempts = 0;
    mock = installTyndMock({
      os: {
        "power.subscribePowerEvents": () => {
          attempts++;
          if (attempts === 1) throw new Error("OS API power is not compiled in this build");
        },
      },
    });
    let fired = 0;
    await expect(power.onLock(() => fired++)).rejects.toThrow("not compiled");
    mock.emitOs("power:lock");
    expect(fired).toBe(0);

    const sub = await power.onLock(() => fired++);
    mock.emitOs("power:lock");
    expect(fired).toBe(1);
    sub.dispose();
    mock.emitOs("power:lock");
    expect(fired).toBe(1);
    expect(attempts).toBe(2);
  });

  test("onPowerSourceChange forwards the payload", async () => {
    mock = installTyndMock({ os: { "power.subscribePowerEvents": () => undefined } });
    const seen: boolean[] = [];
    const sub = await power.onPowerSourceChange(({ onBattery }) => seen.push(onBattery));
    mock.emitOs("power:source-change", { onBattery: true });
    sub.dispose();
    mock.emitOs("power:source-change", { onBattery: false });
    expect(seen).toEqual([true]);
  });

  test("getThermalState maps to power.getThermalState", async () => {
    mock = installTyndMock({ os: { "power.getThermalState": () => "serious" } });
    expect(await power.getThermalState()).toBe("serious");
  });
});
