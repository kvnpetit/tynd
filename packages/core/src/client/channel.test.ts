import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { Channel as BackendChannel } from "../runtime/channel";
import { installTyndMock, type TyndMock } from "../testing";
import { createBackend } from "./backend";
import { Channel, encodeChannelArgs } from "./channel";

let mock: TyndMock;
beforeEach(() => {
  mock = installTyndMock();
});
afterEach(() => mock.restore());

describe("Channel", () => {
  test("delivers messages in order, then closes on the end frame", () => {
    const seen: number[] = [];
    let closed = 0;
    const channel = new Channel<number>((n) => seen.push(n));
    channel.onclose = () => closed++;
    const event = `__tynd_ch__:${channel.id}`;
    mock.emit(event, { m: 1 });
    mock.emit(event, { m: 2 });
    mock.emit(event, { end: true });
    mock.emit(event, { m: 3 });
    expect(seen).toEqual([1, 2]);
    expect(closed).toBe(1);
    expect(channel.closed).toBe(true);
  });

  test("close() stops listening and is idempotent", () => {
    const seen: string[] = [];
    const channel = new Channel<string>((m) => seen.push(m));
    channel.close();
    channel.close();
    mock.emit(`__tynd_ch__:${channel.id}`, { m: "late" });
    expect(seen).toEqual([]);
  });

  test("top-level channel arguments travel as placeholders", () => {
    const channel = new Channel();
    expect(encodeChannelArgs([1, channel])).toEqual([1, { __tynd_channel__: channel.id }]);
    const plain = [1, "x"];
    expect(encodeChannelArgs(plain)).toBe(plain);
  });

  test("a backend Channel<T> parameter takes a frontend Channel<T>", async () => {
    let received: unknown[] = [];
    mock.setBackend("watch", (...args: unknown[]) => {
      received = args;
      return "ok";
    });
    type Api = { watch(path: string, feed: BackendChannel<number>): Promise<string> };
    const channel = new Channel<number>();
    expect(await createBackend<Api>().watch("/tmp", channel)).toBe("ok");
    expect(received).toEqual(["/tmp", { __tynd_channel__: channel.id }]);
  });
});
