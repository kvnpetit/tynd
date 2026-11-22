import { describe, expect, test } from "bun:test";
import { ActionError, DebugClient, type Transport } from "./client.ts";
import { encodeFrame, FrameDecoder, RpcError } from "./protocol.ts";

interface SentRequest {
  jsonrpc: string;
  id: string;
  method: string;
  params: Record<string, unknown>;
}

/** In-memory transport that records requests and lets a test script the replies. */
function mockTransport(handler: (request: SentRequest) => unknown = () => ({ ok: true })) {
  const decoder = new FrameDecoder();
  const sent: SentRequest[] = [];
  let onData: (chunk: Uint8Array) => void = () => {};
  let onClose: (error?: Error) => void = () => {};
  let closed = false;
  const reply = (message: unknown) =>
    onData(encodeFrame(new TextEncoder().encode(JSON.stringify(message))));
  const transport: Transport = {
    write(data) {
      for (const frame of decoder.push(data)) {
        const request = JSON.parse(new TextDecoder().decode(frame)) as SentRequest;
        sent.push(request);
        const result = handler(request);
        if (result instanceof Error) {
          reply({
            jsonrpc: "2.0",
            id: request.id,
            error: { code: -32603, message: result.message },
          });
        } else if (result !== undefined) {
          reply({ jsonrpc: "2.0", id: request.id, result });
        }
      }
    },
    onData: (listener) => {
      onData = listener;
    },
    onClose: (listener) => {
      onClose = listener;
    },
    close: () => {
      closed = true;
    },
  };
  return {
    transport,
    sent,
    reply,
    drop: (error?: Error) => onClose(error),
    get closed() {
      return closed;
    },
  };
}

describe("DebugClient requests", () => {
  test("sends monotonically increasing string ids and resolves with the result", async () => {
    const mock = mockTransport((request) => ({ echoed: request.method }));
    const client = new DebugClient(mock.transport);
    expect(await client.call<unknown>("app.info")).toEqual({ echoed: "app.info" });
    expect(await client.call<unknown>("window.list")).toEqual({ echoed: "window.list" });
    expect(mock.sent.map((request) => request.id)).toEqual(["1", "2"]);
    expect(mock.sent[0]?.jsonrpc).toBe("2.0");
  });

  test("matches out-of-order responses to their requests", async () => {
    const mock = mockTransport(() => undefined);
    const client = new DebugClient(mock.transport);
    const first = client.call("a");
    const second = client.call("b");
    mock.reply({ jsonrpc: "2.0", id: "2", result: "second" });
    mock.reply({ jsonrpc: "2.0", id: "1", result: "first" });
    expect(await first).toBe("first");
    expect(await second).toBe("second");
  });

  test("rejects with RpcError when the host answers with an error", async () => {
    const mock = mockTransport(() => new Error("boom"));
    const client = new DebugClient(mock.transport);
    const error = await client.call("webview.eval").catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(RpcError);
    expect((error as RpcError).code).toBe(-32603);
    expect((error as RpcError).message).toContain("webview.eval");
  });

  test("times out when the host never answers", async () => {
    const mock = mockTransport(() => undefined);
    const client = new DebugClient(mock.transport, { requestTimeout: 20 });
    await expect(client.call("slow")).rejects.toThrow("timed out after 20 ms");
  });

  test("rejects pending and later calls once the connection drops", async () => {
    const mock = mockTransport(() => undefined);
    const client = new DebugClient(mock.transport);
    const pending = client.call("a");
    mock.drop(new Error("pipe broken"));
    await expect(pending).rejects.toThrow("pipe broken");
    await expect(client.call("b")).rejects.toThrow("pipe broken");
    expect(client.closed).toBe(true);
  });

  test("an unsolicited error frame (already_attached) fails the client", async () => {
    const mock = mockTransport(() => undefined);
    const client = new DebugClient(mock.transport);
    const pending = client.call("a");
    mock.reply({ jsonrpc: "2.0", id: null, error: { code: -32099, message: "already_attached" } });
    await expect(pending).rejects.toThrow("already_attached");
  });

  test("delivers notifications to listeners", async () => {
    const mock = mockTransport(() => undefined);
    const client = new DebugClient(mock.transport);
    const seen: [string, unknown][] = [];
    const stop = client.onNotification((method, params) => seen.push([method, params]));
    mock.reply({ jsonrpc: "2.0", method: "ipc.frame", params: { id: "x" } });
    stop();
    mock.reply({ jsonrpc: "2.0", method: "ipc.frame", params: { id: "y" } });
    expect(seen).toEqual([["ipc.frame", { id: "x" }]]);
  });

  test("close() closes the transport and rejects pending calls", async () => {
    const mock = mockTransport(() => undefined);
    const client = new DebugClient(mock.transport);
    const pending = client.call("a");
    client.close();
    await expect(pending).rejects.toThrow("closed");
    expect(mock.closed).toBe(true);
  });
});

describe("DebugClient handshake", () => {
  test("hello sends bridge.hello with the cookie and checks the echo", async () => {
    const mock = mockTransport((request) => ({ cookie: request.params.cookie }));
    await new DebugClient(mock.transport).hello("secret");
    expect(mock.sent[0]).toMatchObject({ method: "bridge.hello", params: { cookie: "secret" } });
  });

  test("hello fails when the echoed cookie differs", async () => {
    const mock = mockTransport(() => ({ cookie: "other" }));
    await expect(new DebugClient(mock.transport).hello("secret")).rejects.toThrow("echo");
  });
});

describe("DebugClient typed methods", () => {
  test("windowList / appInfo / ipcRecent map to their debug methods", async () => {
    const mock = mockTransport((request) => (request.method === "ipc.recent" ? [] : { ok: 1 }));
    const client = new DebugClient(mock.transport);
    await client.windowList();
    await client.appInfo();
    await client.ipcRecent(5);
    expect(mock.sent.map((request) => request.method)).toEqual([
      "window.list",
      "app.info",
      "ipc.recent",
    ]);
    expect(mock.sent[2]?.params).toEqual({ n: 5 });
  });

  test("eval forwards script and label, and parses the JSON text result", async () => {
    const mock = mockTransport(() => '{"a":1}');
    const client = new DebugClient(mock.transport);
    expect(await client.eval<unknown>("1", { label: "settings" })).toEqual({ a: 1 });
    expect(mock.sent[0]?.params).toEqual({ script: "1", label: "settings" });
    expect(await client.eval<unknown>("1", { raw: true })).toBe('{"a":1}');
  });

  test("eval returns text that is not JSON as is", async () => {
    const mock = mockTransport(() => "not json");
    expect(await new DebugClient(mock.transport).eval<unknown>("x")).toBe("not json");
  });

  test("click resolves a selector to a ref through webview.find_element", async () => {
    const mock = mockTransport((request) =>
      request.method === "webview.find_element"
        ? { ref: "e4", x: 0, y: 0, w: 1, h: 1 }
        : { ok: true },
    );
    await new DebugClient(mock.transport).click("#go");
    expect(mock.sent.map((request) => request.method)).toEqual([
      "webview.find_element",
      "webview.click",
    ]);
    expect(mock.sent[0]?.params).toEqual({ selector: "#go" });
    expect(mock.sent[1]?.params).toEqual({ target: { ref: "e4" } });
  });

  test("click passes coordinates through without a lookup", async () => {
    const mock = mockTransport(() => ({ ok: true }));
    await new DebugClient(mock.transport).click({ x: 3, y: 4 });
    expect(mock.sent).toHaveLength(1);
    expect(mock.sent[0]?.params).toEqual({ target: { x: 3, y: 4 } });
  });

  test("click throws when the selector matches nothing", async () => {
    const mock = mockTransport(() => null);
    await expect(new DebugClient(mock.transport).click("#missing")).rejects.toThrow(ActionError);
  });

  test("click surfaces a helper failure", async () => {
    const mock = mockTransport(() => ({ ok: false, error: "ref not found" }));
    await expect(new DebugClient(mock.transport).click({ ref: "e9" })).rejects.toThrow(
      "ref not found",
    );
  });

  test("type sends text with an optional resolved target", async () => {
    const mock = mockTransport((request) =>
      request.method === "webview.find_element" ? { ref: "e2" } : { ok: true },
    );
    const client = new DebugClient(mock.transport);
    await client.type("hi");
    await client.type("yo", "input");
    const typed = mock.sent.filter((request) => request.method === "webview.type");
    expect(typed[0]?.params).toEqual({ text: "hi" });
    expect(typed[1]?.params).toEqual({ text: "yo", target: { ref: "e2" } });
  });

  test("fill resolves selectors and keeps explicit refs", async () => {
    const mock = mockTransport((request) =>
      request.method === "webview.find_element" ? { ref: "e7" } : { ok: true, filled: [] },
    );
    await new DebugClient(mock.transport).fill([
      { selector: "#name", value: "Ada" },
      { ref: "e1", value: "x" },
    ]);
    const fill = mock.sent.find((request) => request.method === "webview.fill");
    expect(fill?.params).toEqual({
      fields: [
        { ref: "e7", value: "Ada" },
        { ref: "e1", value: "x" },
      ],
    });
  });

  test("snapshot defaults to the map mode", async () => {
    const mock = mockTransport(() => ({ map: [], viewport: { w: 1, h: 1 } }));
    await new DebugClient(mock.transport).snapshot();
    await new DebugClient(mock.transport).snapshot({ mode: "state", label: "w2" });
    expect(mock.sent[0]?.params).toEqual({ mode: "map" });
    expect(mock.sent[1]?.params).toEqual({ mode: "state", label: "w2" });
  });

  test("screenshot decodes the base64 PNG payload", async () => {
    const mock = mockTransport(() => ({
      width: 2,
      height: 1,
      png_base64: Buffer.from([137, 80, 78, 71]).toString("base64"),
    }));
    const shot = await new DebugClient(mock.transport).screenshot({ label: "main" });
    expect([...shot.png]).toEqual([137, 80, 78, 71]);
    expect(shot.width).toBe(2);
    expect(mock.sent[0]).toMatchObject({ method: "screenshot.window", params: { label: "main" } });
  });
});
