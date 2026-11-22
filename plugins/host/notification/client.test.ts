import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { installTyndMock, type TyndMock } from "@tynd/core/testing";
import { notification } from "./client";

let bridge: TyndMock;
beforeEach(() => {
  bridge = installTyndMock({ osFallback: () => null });
});
afterEach(() => bridge.restore());

describe("@tynd/plugin-notification client", () => {
  test("send fills an empty body and resolves the id", async () => {
    bridge.setOs("notification", "send", () => ({ id: 7 }));
    expect(await notification.send("Hi")).toBe(7);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "notification", method: "send", args: { title: "Hi", body: "" } },
    ]);
  });

  test("cancel", async () => {
    await notification.cancel(3);
    expect(bridge.calls).toEqual([
      { kind: "os", api: "notification", method: "cancel", args: { id: 3 } },
    ]);
  });

  test("checkPermission", async () => {
    await notification.checkPermission();
    expect(bridge.calls).toEqual([
      { kind: "os", api: "notification", method: "checkPermission", args: null },
    ]);
  });
});

describe("@tynd/plugin-notification options and events", () => {
  test("send forwards every option", async () => {
    bridge.setOs("notification", "send", () => ({ id: 1 }));
    await notification.send("Hi", {
      body: "b",
      image: "/img.png",
      silent: true,
      urgency: "critical",
      timeout: "never",
      category: "invite",
      reply: { placeholder: "Answer" },
    });
    expect((bridge.calls[0] as { args: unknown }).args).toEqual({
      title: "Hi",
      body: "b",
      image: "/img.png",
      silent: true,
      urgency: "critical",
      timeout: "never",
      category: "invite",
      reply: { placeholder: "Answer" },
    });
  });

  test("registerActions, remove and removeAll", async () => {
    await notification.registerActions("invite", [{ id: "yes", label: "Yes" }]);
    await notification.remove(4);
    await notification.removeAll();
    expect(
      bridge.calls.map((c) => [(c as { method: string }).method, (c as { args: unknown }).args]),
    ).toEqual([
      ["registerActions", { category: "invite", actions: [{ id: "yes", label: "Yes" }] }],
      ["remove", { id: 4 }],
      ["removeAll", null],
    ]);
  });
});

describe("@tynd/plugin-notification events", () => {
  test("event helpers subscribe to their own event", () => {
    const seen: unknown[] = [];
    const offs = [
      notification.onClick((e) => seen.push(["click", e])),
      notification.onAction((e) => seen.push(["action", e])),
      notification.onReply((e) => seen.push(["reply", e])),
      notification.onClose((e) => seen.push(["close", e])),
    ];
    bridge.emitOs("notification:click", { id: 1 });
    bridge.emitOs("notification:action", { id: 1, action: "yes" });
    bridge.emitOs("notification:reply", { id: 1, text: "hi" });
    bridge.emitOs("notification:close", { id: 1, reason: "user" });
    for (const off of offs) off();
    bridge.emitOs("notification:click", { id: 2 });
    expect(seen).toEqual([
      ["click", { id: 1 }],
      ["action", { id: 1, action: "yes" }],
      ["reply", { id: 1, text: "hi" }],
      ["close", { id: 1, reason: "user" }],
    ]);
  });
});
