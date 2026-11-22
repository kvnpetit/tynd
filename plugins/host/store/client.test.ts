import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { createStore, type StoreErrorEvent } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-store client", () => {
  test("set is scoped to the namespace", async () => {
    await createStore("prefs").set("k", 1);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "store", method: "set", args: { namespace: "prefs", key: "k", value: 1 } },
    ]);
  });
});

// Tests below drive the client through the full core mock (os_on events, host errors).
let mock: TyndMock | undefined;

describe("store", () => {
  afterEach(() => {
    mock?.restore();
    mock = undefined;
  });

  test("flush calls the host for its namespace and propagates a failed write", async () => {
    mock = installTyndMock({
      os: {
        "store.flush": () => {
          throw new Error("disk full");
        },
      },
    });
    const prefs = createStore("com.example.prefs");
    await expect(prefs.flush()).rejects.toThrow("disk full");
    expect(mock.calls).toContainEqual({
      kind: "os",
      api: "store",
      method: "flush",
      args: { namespace: "com.example.prefs" },
    });
  });

  test("onError delivers store:error for its own namespace only, until unsubscribed", () => {
    mock = installTyndMock();
    const seen: StoreErrorEvent[] = [];
    const off = createStore("a").onError((event) => seen.push(event));
    const failure = { namespace: "a", phase: "touch", path: "/x/store.json", error: "EROFS" };
    mock.emitOs("store:error", { ...failure, namespace: "b" });
    mock.emitOs("store:error", failure);
    off();
    mock.emitOs("store:error", failure);
    expect(seen).toEqual([failure as StoreErrorEvent]);
  });
});
