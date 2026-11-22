import { afterEach, describe, expect, test } from "bun:test";
import {
  bindShortcut,
  clear,
  close,
  isOpen,
  list,
  onOpenChange,
  register,
  registerMany,
  run,
  search,
  toggle,
  unregister,
} from "./client";

afterEach(() => {
  clear();
  close();
});

const noop = () => undefined;

function fakeTarget() {
  let handler: ((e: unknown) => void) | null = null;
  const target = {
    addEventListener(_type: string, h: (e: unknown) => void) {
      handler = h;
    },
    removeEventListener(_type: string, h: (e: unknown) => void) {
      if (handler === h) handler = null;
    },
  };
  return {
    target: target as unknown as EventTarget,
    fire: (event: Record<string, unknown>) => {
      const calls = { prevented: false };
      handler?.({
        ...event,
        preventDefault: () => {
          calls.prevented = true;
        },
      });
      return calls;
    },
    hasHandler: () => handler !== null,
  };
}

describe("registry", () => {
  test("register overwrites by id, unregister is a no-op for unknown ids", () => {
    register({ id: "a", title: "A", action: noop });
    register({ id: "a", title: "A2", action: noop });
    unregister("missing");
    expect(list().map((c) => c.title)).toEqual(["A2"]);
    registerMany([
      { id: "b", title: "B", action: noop },
      { id: "a", title: "A3", action: noop },
    ]);
    expect(list().map((c) => c.title)).toEqual(["A3", "B"]);
  });
});

describe("search", () => {
  test("ranks exact, prefix, contains, keyword, subsequence", () => {
    registerMany([
      { id: "sub", title: "open file", action: noop },
      { id: "kw", title: "Zeta", keywords: ["open"], action: noop },
      { id: "has", title: "reopen", action: noop },
      { id: "pre", title: "Open Recent", action: noop },
      { id: "exact", title: "open", action: noop },
    ]);
    const ids = search("open").map((r) => r.id);
    expect(ids).toEqual(["exact", "sub", "pre", "has", "kw"]);
    expect(search("opf").map((r) => r.id)).toEqual(["sub"]);
    expect(search("zzz")).toEqual([]);
  });

  test("empty query returns everything in insertion order, honours limit", () => {
    registerMany([
      { id: "1", title: "one", action: noop },
      { id: "2", title: "two", action: noop },
      { id: "3", title: "three", action: noop },
    ]);
    expect(search("  ").map((r) => r.id)).toEqual(["1", "2", "3"]);
    expect(search("", 2)).toHaveLength(2);
  });

  test("when() hides commands and a throwing when() does not abort the search", () => {
    registerMany([
      { id: "hidden", title: "hidden", when: () => false, action: noop },
      {
        id: "boom",
        title: "boom",
        when: () => {
          throw new Error("bad predicate");
        },
        action: noop,
      },
      { id: "ok", title: "ok", action: noop },
    ]);
    expect(search("").map((r) => r.id)).toEqual(["ok"]);
  });
});

describe("run", () => {
  test("runs the action and closes the palette, even when the action throws", async () => {
    let ran = 0;
    register({ id: "a", title: "A", action: () => void ran++ });
    register({
      id: "bad",
      title: "Bad",
      action: () => {
        throw new Error("nope");
      },
    });
    toggle();
    expect(isOpen()).toBe(true);
    expect(await run("a")).toBe(true);
    expect(ran).toBe(1);
    expect(isOpen()).toBe(false);

    toggle();
    await expect(run("bad")).rejects.toThrow("nope");
    expect(isOpen()).toBe(false);
  });

  test("unknown, hidden and throwing-when commands do not run", async () => {
    let ran = false;
    const markRan = () => {
      ran = true;
    };
    register({ id: "h", title: "h", when: () => false, action: () => markRan() });
    register({
      id: "t",
      title: "t",
      when: () => {
        throw new Error("x");
      },
      action: () => markRan(),
    });
    expect(await run("missing")).toBe(false);
    expect(await run("h")).toBe(false);
    expect(await run("t")).toBe(false);
    expect(ran).toBe(false);
  });
});

describe("open state", () => {
  test("notifies listeners once per change and survives a throwing listener", () => {
    const seen: boolean[] = [];
    const off = onOpenChange(() => {
      throw new Error("listener");
    });
    const off2 = onOpenChange((open) => seen.push(open));
    toggle();
    toggle();
    close();
    expect(seen).toEqual([true, false]);
    off();
    off2();
  });
});

describe("bindShortcut", () => {
  test("Ctrl+K toggles, plain k does not, and preventDefault is called on match", () => {
    const { target, fire } = fakeTarget();
    const off = bindShortcut({ target });
    expect(fire({ key: "k", ctrlKey: true }).prevented).toBe(true);
    expect(isOpen()).toBe(true);
    expect(fire({ key: "K", metaKey: true }).prevented).toBe(true);
    expect(isOpen()).toBe(false);
    expect(fire({ key: "k" }).prevented).toBe(false);
    expect(isOpen()).toBe(false);
    off();
  });

  test("an event without a key (autofill) is ignored instead of throwing", () => {
    const { target, fire } = fakeTarget();
    const off = bindShortcut({ target });
    expect(() => fire({ ctrlKey: true })).not.toThrow();
    expect(isOpen()).toBe(false);
    off();
  });

  test("bare-key bindings ignore typing in inputs but fire elsewhere", () => {
    const { target, fire } = fakeTarget();
    let triggers = 0;
    const off = bindShortcut({
      target,
      key: "/",
      metaOrCtrl: false,
      onTrigger: () => void triggers++,
    });
    const typing = fire({ key: "/", target: { tagName: "INPUT" } });
    expect(typing.prevented).toBe(false);
    expect(fire({ key: "/", target: { tagName: "textarea" } }).prevented).toBe(false);
    expect(fire({ key: "/", target: { tagName: "DIV", isContentEditable: true } }).prevented).toBe(
      false,
    );
    expect(triggers).toBe(0);
    expect(fire({ key: "/", target: { tagName: "BODY" } }).prevented).toBe(true);
    expect(triggers).toBe(1);
    off();
  });

  test("rebinding drops the previous listener and unsubscribe detaches", () => {
    const first = fakeTarget();
    const second = fakeTarget();
    bindShortcut({ target: first.target });
    const off = bindShortcut({ target: second.target });
    expect(first.hasHandler()).toBe(false);
    expect(second.hasHandler()).toBe(true);
    off();
    expect(second.hasHandler()).toBe(false);
  });
});
