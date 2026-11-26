import { afterEach, describe, expect, test } from "bun:test";
import { convertFileSrc, protocolUrl } from "./protocol";

const g = globalThis as Record<string, unknown>;

function withBinOrigin(origin: string, run: () => void): void {
  g.__TYND_BIN_ORIGIN__ = origin;
  try {
    run();
  } finally {
    Reflect.deleteProperty(g, "__TYND_BIN_ORIGIN__");
  }
}

afterEach(() => {
  Reflect.deleteProperty(g, "__TYND_BIN_ORIGIN__");
});

describe("convertFileSrc", () => {
  test("encodes the whole path as one segment of the asset origin", () => {
    withBinOrigin("tynd-bin://localhost", () => {
      expect(convertFileSrc("/home/me/My Videos/clip#1.mp4")).toBe(
        "asset://localhost/%2Fhome%2Fme%2FMy%20Videos%2Fclip%231.mp4",
      );
    });
  });

  test("uses the http .localhost spelling where the webview needs it (WebView2)", () => {
    withBinOrigin("http://tynd-bin.localhost", () => {
      expect(convertFileSrc("C:\\Users\\me\\a b.mp4")).toBe(
        "http://asset.localhost/C%3A%5CUsers%5Cme%5Ca%20b.mp4",
      );
    });
  });

  test("rejects an empty or non-string path", () => {
    expect(() => convertFileSrc("")).toThrow(TypeError);
    expect(() => convertFileSrc(undefined as unknown as string)).toThrow(TypeError);
  });
});

describe("protocolUrl", () => {
  test("joins the scheme origin and the path as written", () => {
    withBinOrigin("tynd-bin://localhost", () => {
      expect(protocolUrl("thumbs", "/a%20b?size=128")).toBe("thumbs://localhost/a%20b?size=128");
      expect(protocolUrl("thumbs", "x")).toBe("thumbs://localhost/x");
      expect(protocolUrl("thumbs")).toBe("thumbs://localhost/");
    });
    withBinOrigin("http://tynd-bin.localhost", () => {
      expect(protocolUrl("my-media", "/v/1")).toBe("http://my-media.localhost/v/1");
    });
  });

  test("rejects names the host would refuse", () => {
    for (const bad of ["Media", "1x", "a_b", "x-", ""]) {
      expect(() => protocolUrl(bad)).toThrow("not a valid protocol name");
    }
  });
});
