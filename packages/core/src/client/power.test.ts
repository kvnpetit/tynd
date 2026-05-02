import { afterEach, beforeEach, describe, expect, test } from "bun:test";

type OsCall = { api: string; method: string; args: unknown };
type EventHandler = (data: unknown) => void;

interface Shim {
  osCalls: OsCall[];
  emit(name: string, data?: unknown): void;
  listeners(name: string): EventHandler[];
}

function mountShim(): Shim {
  const osCalls: OsCall[] = [];
  const eventListeners: Map<string, EventHandler[]> = new Map();

  (globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        osCalls.push({ api, method, args });
        return Promise.resolve(null);
      },
      os_on(name: string, handler: EventHandler) {
        const arr = eventListeners.get(name) ?? [];
        arr.push(handler);
        eventListeners.set(name, arr);
        return () => {
          const list = eventListeners.get(name);
          if (!list) return;
          const idx = list.indexOf(handler);
          if (idx >= 0) list.splice(idx, 1);
        };
      },
      call() {
        return Promise.resolve(null);
      },
      on() {
        return () => undefined;
      },
      off() {
        return undefined;
      },
    },
  };
  return {
    osCalls,
    emit(name, data) {
      const list = eventListeners.get(name);
      if (!list) return;
      for (const h of list.slice()) h(data);
    },
    listeners(name) {
      return eventListeners.get(name) ?? [];
    },
  };
}

// Power module caches a "subscribed once" flag in module scope; reload it
// per test so each describe block sees a clean lazy-subscribe path.
async function importFreshPower() {
  const path = `./power?${Math.random().toString(36).slice(2)}`;
  // Bun supports `import()` with a cache-busting query string; the cast
  // bypasses TS's static module-graph view since the runtime resolution
  // is identical to `./power` modulo the query.
  const mod = (await import(/* @vite-ignore */ path)) as typeof import("./power");
  return mod.power;
}

describe("power events — wire format + lazy subscribe", () => {
  let shim: Shim;
  beforeEach(() => {
    shim = mountShim();
  });
  afterEach(() => {
    (globalThis as { window?: unknown }).window = undefined;
  });

  test("first onSuspend triggers a single subscribePowerEvents dispatch", async () => {
    const power = await importFreshPower();
    const sub = power.onSuspend(() => {});
    // The lazy subscribe is async (osCall returns a promise) — let it settle.
    await Promise.resolve();
    await Promise.resolve();
    expect(shim.osCalls.filter((c) => c.method === "subscribePowerEvents").length).toBe(1);
    sub.dispose();
  });

  test("multiple onSuspend handlers do not re-subscribe", async () => {
    const power = await importFreshPower();
    const a = power.onSuspend(() => {});
    const b = power.onSuspend(() => {});
    const c = power.onLock(() => {});
    await Promise.resolve();
    await Promise.resolve();
    expect(shim.osCalls.filter((c) => c.method === "subscribePowerEvents").length).toBe(1);
    a.dispose();
    b.dispose();
    c.dispose();
  });

  test("emitting power:suspend fires every handler", async () => {
    const power = await importFreshPower();
    let countA = 0;
    let countB = 0;
    const a = power.onSuspend(() => countA++);
    const b = power.onSuspend(() => countB++);
    shim.emit("power:suspend");
    expect(countA).toBe(1);
    expect(countB).toBe(1);
    a.dispose();
    b.dispose();
  });

  test("dispose removes the handler", async () => {
    const power = await importFreshPower();
    let count = 0;
    const sub = power.onResume(() => count++);
    shim.emit("power:resume");
    expect(count).toBe(1);
    sub.dispose();
    shim.emit("power:resume");
    expect(count).toBe(1);
  });

  test("onLock + onUnlock route to the correct event names", async () => {
    const power = await importFreshPower();
    let lockCount = 0;
    let unlockCount = 0;
    const lockSub = power.onLock(() => lockCount++);
    const unlockSub = power.onUnlock(() => unlockCount++);
    shim.emit("power:lock");
    shim.emit("power:lock");
    shim.emit("power:unlock");
    expect(lockCount).toBe(2);
    expect(unlockCount).toBe(1);
    lockSub.dispose();
    unlockSub.dispose();
  });
});

describe("power getters — kept stable across W7.3", () => {
  let shim: Shim;
  beforeEach(() => {
    shim = mountShim();
  });
  afterEach(() => {
    (globalThis as { window?: unknown }).window = undefined;
  });

  test("getIdleTime + getPowerStatus still route through power.* dispatch", async () => {
    const power = await importFreshPower();
    // Each shim call resolves with null — getIdleTime destructures so swallow
    // the rejection to keep the test focused on the wire-format check.
    power.getIdleTime().catch(() => {});
    power.getPowerStatus().catch(() => {});
    // Wait one microtask for the dispatches to fire before asserting.
    await Promise.resolve();
    expect(shim.osCalls.map((c) => `${c.api}.${c.method}`)).toEqual([
      "power.getIdleTime",
      "power.getPowerStatus",
    ]);
  });
});
