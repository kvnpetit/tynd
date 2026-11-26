import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type MockOsCall, type TyndMock } from "../testing";
import { os } from "./os";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

const osCalls = () => bridge.calls.filter((call): call is MockOsCall => call.kind === "os");

describe("os regional settings", () => {
  test("region, languages, timezone and clock format map to their host methods", async () => {
    bridge.setOs("os", "languages", () => ["fr-FR", "en-US"]);
    bridge.setOs("os", "uses24HourClock", () => true);
    await os.region();
    expect(await os.languages()).toEqual(["fr-FR", "en-US"]);
    await os.timezone();
    expect(await os.uses24HourClock()).toBe(true);
    expect(osCalls().map((c) => c.method)).toEqual([
      "region",
      "languages",
      "timezone",
      "uses24HourClock",
    ]);
  });
});

describe("os media access", () => {
  test("status and prompt send the media kind", async () => {
    bridge.setOs("os", "getMediaAccessStatus", () => "not-determined");
    bridge.setOs("os", "askForMediaAccess", () => true);
    expect(await os.getMediaAccessStatus("camera")).toBe("not-determined");
    expect(await os.askForMediaAccess("microphone")).toBe(true);
    expect(osCalls().map(({ method, args }) => ({ method, args }))).toEqual([
      { method: "getMediaAccessStatus", args: { kind: "camera" } },
      { method: "askForMediaAccess", args: { kind: "microphone" } },
    ]);
  });
});

describe("os app directories", () => {
  test("each helper maps to its host method and returns the path", async () => {
    for (const method of [
      "appDataDir",
      "appConfigDir",
      "appCacheDir",
      "appTempDir",
      "appLogDir",
    ] as const) {
      bridge.setOs("os", method, () => `/data/com.example.app/${method}`);
    }
    expect(await os.appDataDir()).toBe("/data/com.example.app/appDataDir");
    expect(await os.appConfigDir()).toBe("/data/com.example.app/appConfigDir");
    expect(await os.appCacheDir()).toBe("/data/com.example.app/appCacheDir");
    expect(await os.appTempDir()).toBe("/data/com.example.app/appTempDir");
    expect(await os.appLogDir()).toBe("/data/com.example.app/appLogDir");
    expect(osCalls().map((c) => c.method)).toEqual([
      "appDataDir",
      "appConfigDir",
      "appCacheDir",
      "appTempDir",
      "appLogDir",
    ]);
  });
});
