import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { http } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-http client", () => {
  test("getJson maps to a json request", async () => {
    await http.getJson("https://x.test");
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "http",
        method: "request",
        args: { url: "https://x.test", method: "GET", responseType: "json" },
      },
    ]);
  });

  test("post maps to a POST request", async () => {
    await http.post("https://x.test", { body: { a: 1 } });
    expect(bridge.calls).toEqual([
      {
        kind: "os",
        api: "http",
        method: "request",
        args: {
          url: "https://x.test",
          method: "POST",
          responseType: "text",
          body: { a: 1 },
        },
      },
    ]);
  });
});
