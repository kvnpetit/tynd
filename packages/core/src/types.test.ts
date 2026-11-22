import { describe, expect, test } from "bun:test";
import * as v from "valibot";
import {
  AppConfigSchema,
  DOWNLOAD_POLICIES,
  NEW_WINDOW_POLICIES,
  type ObservabilityConfig,
  PROXY_URL_PATTERN,
  WEB_PERMISSION_DECISIONS,
  type WindowConfig,
} from "./types";

const parse = (window: unknown) => v.safeParse(AppConfigSchema, { window });

describe("window.downloads", () => {
  test("accepts allow and deny only", () => {
    expect([...DOWNLOAD_POLICIES]).toEqual(["allow", "deny"]);
    for (const policy of DOWNLOAD_POLICIES) {
      expect(parse({ downloads: policy }).success).toBe(true);
    }
    expect(parse({ downloads: "ask" }).success).toBe(false);
    expect(parse({}).success).toBe(true);
  });
});

describe("window.permissions", () => {
  test("accepts a decision per known kind", () => {
    expect([...WEB_PERMISSION_DECISIONS]).toEqual(["allow", "deny", "prompt"]);
    expect(
      parse({ permissions: { camera: "allow", microphone: "prompt", geolocation: "deny" } })
        .success,
    ).toBe(true);
    expect(parse({ permissions: {} }).success).toBe(true);
  });

  test("rejects unknown kinds and decisions", () => {
    expect(parse({ permissions: { webcam: "allow" } }).success).toBe(false);
    expect(parse({ permissions: { camera: "always" } }).success).toBe(false);
  });
});

describe("window.visible / kiosk / titleBarStyle", () => {
  test("accept booleans and the three macOS title bar styles", () => {
    expect(parse({ visible: false, kiosk: true }).success).toBe(true);
    for (const style of ["visible", "transparent", "overlay"]) {
      expect(parse({ titleBarStyle: style }).success).toBe(true);
    }
    expect(parse({ titleBarStyle: "hiddenInset" }).success).toBe(false);
    expect(parse({ visible: "no" }).success).toBe(false);
  });
});

describe("window.newWindow", () => {
  test("accepts every documented policy", () => {
    for (const policy of NEW_WINDOW_POLICIES) {
      expect(parse({ newWindow: policy }).success).toBe(true);
    }
    expect([...NEW_WINDOW_POLICIES]).toEqual(["deny", "external", "allow"]);
  });

  test("rejects unknown policies", () => {
    expect(parse({ newWindow: "popup" }).success).toBe(false);
    expect(parse({ newWindow: "" }).success).toBe(false);
  });

  test("is optional", () => {
    expect(parse({}).success).toBe(true);
    const typed: WindowConfig = { newWindow: "deny", proxy: "socks5://127.0.0.1:9050" };
    expect(typed.newWindow).toBe("deny");
  });
});

describe("window.proxy", () => {
  test("accepts http and socks5 endpoints", () => {
    for (const proxy of [
      "http://proxy.local:3128",
      "http://proxy.local",
      "http://10.0.0.1:8080/",
      "socks5://127.0.0.1:1080",
    ]) {
      expect(parse({ proxy }).success).toBe(true);
      expect(PROXY_URL_PATTERN.test(proxy)).toBe(true);
    }
  });

  test("rejects credentials, paths, ports-less socks5 and other schemes", () => {
    for (const proxy of [
      "http://user:pw@proxy.local:3128",
      "http://proxy.local:3128/path",
      "socks5://proxy.local",
      "https://proxy.local:443",
      "proxy.local:3128",
      "",
    ]) {
      expect(parse({ proxy }).success).toBe(false);
    }
  });
});

describe("ObservabilityConfig", () => {
  test("types the two pipelines", () => {
    const config: ObservabilityConfig = {
      sentry: { dsn: "https://k@o.example.com/1", sampleRate: 0.5 },
      otel: { endpoint: "http://localhost:4317", serviceName: "app" },
    };
    expect(config.sentry?.sampleRate).toBe(0.5);
    // @ts-expect-error a Sentry pipeline needs a dsn
    const missingDsn: ObservabilityConfig = { sentry: { environment: "prod" } };
    expect(missingDsn).toBeDefined();
  });
});
