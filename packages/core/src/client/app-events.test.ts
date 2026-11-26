import { describe, expect, test } from "bun:test";
import { createChannel } from "./app-events";

class FakeSource<T> {
  emit: (value: T) => void = () => undefined;
  drains = 0;
  listens = 0;

  private readonly drained: T[] | Error;

  constructor(drained: T[] | Error = []) {
    this.drained = drained;
  }

  push(value: T): void {
    this.emit(value);
  }

  readonly channel = {
    listen: (next: (value: T) => void) => {
      this.listens++;
      this.emit = next;
    },
    drain: async () => {
      this.drains++;
      if (this.drained instanceof Error) throw this.drained;
      return this.drained;
    },
  };
}

function source<T>(drained: T[] | Error = []): FakeSource<T> {
  return new FakeSource(drained);
}

const ignore = (): void => undefined;

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("createChannel", () => {
  test("delivers the cold-start buffer to the first subscriber, then live events", async () => {
    const src = source<string>(["a.txt", "b.txt"]);
    const subscribe = createChannel(src.channel);
    const seen: string[] = [];
    subscribe((path) => seen.push(path));
    await flush();
    src.push("c.txt");
    expect(seen).toEqual(["a.txt", "b.txt", "c.txt"]);
  });

  test("attaches and drains once, however many handlers subscribe", async () => {
    const src = source<string>(["a.txt"]);
    const subscribe = createChannel(src.channel);
    const first: string[] = [];
    const second: string[] = [];
    subscribe((path) => first.push(path));
    subscribe((path) => second.push(path));
    await flush();
    subscribe(ignore);
    await flush();
    expect(src.listens).toBe(1);
    expect(src.drains).toBe(1);
    expect(first).toEqual(["a.txt"]);
    expect(second).toEqual(["a.txt"]);
  });

  test("a failed drain is retried on the next subscription", async () => {
    const failing = source<string>(new Error("no such method"));
    const subscribe = createChannel(failing.channel);
    const seen: string[] = [];
    subscribe((path) => seen.push(path));
    await flush();
    failing.push("live.txt");
    expect(seen).toEqual(["live.txt"]);
    subscribe(ignore);
    await flush();
    expect(failing.drains).toBe(2);
  });

  test("dispose stops delivery", async () => {
    const src = source<string>();
    const subscribe = createChannel(src.channel);
    const seen: string[] = [];
    const sub = subscribe((path) => seen.push(path));
    await flush();
    src.push("a");
    sub.dispose();
    src.push("b");
    expect(seen).toEqual(["a"]);
  });
});

describe("createChannel without subscribers or with failing handlers", () => {
  test("holdWhenIdle keeps events for the next subscriber", async () => {
    const src = source<string>(["cold.txt"]);
    const subscribe = createChannel({ ...src.channel, holdWhenIdle: true });
    const first = subscribe(ignore);
    await flush();
    first.dispose();
    src.push("while-idle.txt");

    const seen: string[] = [];
    subscribe((path) => seen.push(path));
    expect(seen).toEqual(["while-idle.txt"]);
  });

  test("events with no handler are dropped by default", () => {
    const src = source<string>();
    const subscribe = createChannel(src.channel);
    subscribe(ignore).dispose();
    src.push("lost");
    const seen: string[] = [];
    subscribe((value) => seen.push(value));
    expect(seen).toEqual([]);
  });

  test("a throwing handler does not starve the others", async () => {
    const src = source<string>();
    const subscribe = createChannel(src.channel);
    const seen: string[] = [];
    subscribe(() => {
      throw new Error("boom");
    });
    subscribe((value) => seen.push(value));
    await flush();
    // biome-ignore lint/suspicious/noConsole: silence the expected handler error log
    const original = console.error;
    console.error = ignore;
    try {
      src.push("x");
    } finally {
      console.error = original;
    }
    expect(seen).toEqual(["x"]);
  });
});
