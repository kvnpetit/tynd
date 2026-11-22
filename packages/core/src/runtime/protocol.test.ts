import { afterEach, describe, expect, spyOn, test } from "bun:test";
import * as v from "valibot";
import { AppConfigSchema } from "../types";
import { handleParsedMsg } from "./bun";
import {
  handleProtocolMsg,
  MAX_PROTOCOL_BODY_BYTES,
  type ProtocolMsg,
  protocolNames,
  registerProtocol,
  resetProtocolsForTests,
} from "./protocol";

interface Reply {
  type: string;
  id: string;
  status: number;
  headers: [string, string][];
  body: string | null;
}

const b64 = (text: string) => Buffer.from(text).toString("base64");
const text = (reply: Reply) => Buffer.from(reply.body ?? "", "base64").toString();

function msg(overrides: Partial<ProtocolMsg> = {}): ProtocolMsg {
  return {
    type: "protocol",
    id: "__tynd_protocol_1",
    scheme: "media",
    method: "GET",
    url: "media://localhost/clip?x=1",
    headers: [["range", "bytes=0-"]],
    body: null,
    webview: "main",
    ...overrides,
  };
}

async function run(m: ProtocolMsg): Promise<Reply> {
  const frames: Reply[] = [];
  await handleProtocolMsg(m, (frame) => frames.push(frame as unknown as Reply));
  expect(frames).toHaveLength(1);
  return frames[0] as Reply;
}

afterEach(() => resetProtocolsForTests());

describe("registerProtocol", () => {
  test("records names for the config frame", () => {
    registerProtocol("media", () => new Response());
    registerProtocol("thumbs-2", () => new Response());
    expect(protocolNames()).toEqual(["media", "thumbs-2"]);
  });

  test("refuses invalid, reserved and duplicate names", () => {
    for (const bad of ["Media", "1x", "a_b", "x-", "", "a".repeat(33)]) {
      expect(() => registerProtocol(bad, () => new Response())).toThrow(
        "not a valid protocol name",
      );
    }
    for (const reserved of ["tynd", "tynd-bin", "asset", "https", "file"]) {
      expect(() => registerProtocol(reserved, () => new Response())).toThrow("reserved");
    }
    registerProtocol("media", () => new Response());
    expect(() => registerProtocol("media", () => new Response())).toThrow("already registered");
    expect(() => registerProtocol("other", "nope" as never)).toThrow("must be a function");
  });
});

describe("handleProtocolMsg", () => {
  test("hands the handler a Request and returns its Response", async () => {
    let seen: { url: string; method: string; range: string | null; webview: string } | undefined;
    registerProtocol("media", (request, ctx) => {
      seen = {
        url: request.url,
        method: request.method,
        range: request.headers.get("range"),
        webview: ctx.webview,
      };
      return new Response("clip", {
        status: 206,
        headers: { "content-type": "video/mp4", "content-range": "bytes 0-3/4" },
      });
    });
    const reply = await run(msg());
    expect(seen).toEqual({
      url: "media://localhost/clip?x=1",
      method: "GET",
      range: "bytes=0-",
      webview: "main",
    });
    expect(reply.type).toBe("protocol_response");
    expect(reply.id).toBe("__tynd_protocol_1");
    expect(reply.status).toBe(206);
    expect(reply.headers).toContainEqual(["content-type", "video/mp4"]);
    expect(reply.headers).toContainEqual(["content-range", "bytes 0-3/4"]);
    expect(text(reply)).toBe("clip");
  });

  test("passes request bodies, never for GET", async () => {
    const bodies: string[] = [];
    registerProtocol("media", async (request) => {
      bodies.push(await request.text());
      return new Response(null, { status: 204 });
    });
    const posted = await run(msg({ method: "POST", body: b64("payload") }));
    expect(posted.status).toBe(204);
    expect(posted.body).toBeNull();
    await run(msg({ method: "GET", body: b64("ignored") }));
    expect(bodies).toEqual(["payload", ""]);
  });
});

describe("handleProtocolMsg failures", () => {
  test("a throwing or wrong handler becomes a 500, an unknown scheme a 404", async () => {
    registerProtocol("boom", () => {
      throw new Error("kaput");
    });
    registerProtocol("wrong", () => "text" as unknown as Response);
    const thrown = await run(msg({ scheme: "boom" }));
    expect(thrown.status).toBe(500);
    expect(text(thrown)).toContain("kaput");
    const wrong = await run(msg({ scheme: "wrong" }));
    expect(wrong.status).toBe(500);
    expect(text(wrong)).toContain("did not return a Response");
    expect((await run(msg({ scheme: "nope" }))).status).toBe(404);
  });

  test("caps the response body", async () => {
    registerProtocol("media", () => new Response(new Uint8Array(MAX_PROTOCOL_BODY_BYTES + 1)));
    const reply = await run(msg());
    expect(reply.status).toBe(502);
    expect(text(reply)).toContain("too large");
  });

  test("drops transfer headers the host owns", async () => {
    registerProtocol("media", () => new Response("x", { headers: { "content-length": "1" } }));
    const reply = await run(msg());
    expect(reply.headers.some(([name]) => name === "content-length")).toBe(false);
  });
});

describe("protocol frames over the wire", () => {
  test("a protocol frame is answered on stdout", async () => {
    registerProtocol("media", () => new Response("ok"));
    const lines: string[] = [];
    const spy = spyOn(process.stdout, "write").mockImplementation(((chunk: unknown) => {
      lines.push(String(chunk));
      return true;
    }) as never);
    try {
      await handleParsedMsg(msg(), "unused-entry");
    } finally {
      spy.mockRestore();
    }
    const reply = JSON.parse(lines.join("").trim()) as Reply;
    expect(reply.type).toBe("protocol_response");
    expect(text(reply)).toBe("ok");
  });
});

describe("app.start({ assets })", () => {
  const parse = (assets: unknown) => v.safeParse(AppConfigSchema, { assets });

  test("accepts absolute path globs", () => {
    expect(parse({ scope: [] }).success).toBe(true);
    expect(
      parse({ scope: ["/home/me/Videos/**", "C:/Users/me/**", "D:\\Media\\*.mp4"] }).success,
    ).toBe(true);
  });

  test("rejects relative globs, unknown keys and a missing scope", () => {
    expect(parse({ scope: ["**/*.mp4"] }).success).toBe(false);
    expect(parse({ scope: ["Videos/**"] }).success).toBe(false);
    expect(parse({ scope: ["/a"], allow: [] }).success).toBe(false);
    expect(parse({}).success).toBe(false);
  });
});
