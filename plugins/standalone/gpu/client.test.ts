import { afterEach, describe, expect, test } from "bun:test";
import { detectDedicated, parseVendor, tryWebGL } from "./client";
import { releaseWebGLContext } from "./webgl";

const NO_WEBGPU = { supported: false } as const;

describe("parseVendor", () => {
  test("recognises the major vendors from renderer strings", () => {
    expect(parseVendor("", "NVIDIA GeForce RTX 4070")).toBe("NVIDIA");
    expect(parseVendor("", "AMD Radeon RX 7800 XT")).toBe("AMD");
    expect(parseVendor("Apple", "Apple M2")).toBe("Apple");
    expect(parseVendor("", "Intel(R) UHD Graphics 630")).toBe("Intel");
    expect(parseVendor("", "Qualcomm Adreno 740")).toBe("Qualcomm");
    expect(parseVendor("", "Mali-G78")).toBe("ARM");
  });

  test("detects Intel Arc without matching words that merely contain 'arc'", () => {
    expect(parseVendor("", "Intel(R) Arc(TM) A770 Graphics")).toBe("Intel");
    expect(parseVendor("", "Arc A380")).toBe("Intel");
    expect(parseVendor("Search Engines Inc", "")).toBe("Search Engines Inc");
  });

  test("falls back to the raw vendor, then 'unknown'", () => {
    expect(parseVendor("Acme", "Widget 3000")).toBe("Acme");
    expect(parseVendor("", "")).toBe("unknown");
  });
});

describe("detectDedicated", () => {
  test("NVIDIA and Apple are dedicated", () => {
    expect(detectDedicated("NVIDIA", "GeForce RTX 4070", NO_WEBGPU)).toBe(true);
    expect(detectDedicated("Apple", "Apple M2", NO_WEBGPU)).toBe(true);
  });

  test("AMD APUs are not dedicated, discrete cards are", () => {
    expect(detectDedicated("AMD", "AMD Radeon(TM) Graphics", NO_WEBGPU)).toBe(false);
    expect(detectDedicated("AMD", "Radeon Vega 8 Graphics", NO_WEBGPU)).toBe(false);
    expect(detectDedicated("AMD", "AMD Radeon RX 7800 XT", NO_WEBGPU)).toBe(true);
  });

  test("discrete Intel Arc is dedicated, integrated Intel is not", () => {
    expect(detectDedicated("Intel", "Intel(R) Arc(TM) A770 Graphics", NO_WEBGPU)).toBe(true);
    expect(detectDedicated("Intel", "Intel(R) Arc(TM) B580 Graphics", NO_WEBGPU)).toBe(true);
    expect(detectDedicated("Intel", "Intel(R) Arc(TM) Pro A60", NO_WEBGPU)).toBe(true);
    expect(detectDedicated("Intel", "Intel(R) Arc(TM) Graphics", NO_WEBGPU)).toBe(false);
    expect(detectDedicated("Intel", "Intel(R) Iris(R) Xe MAX Graphics", NO_WEBGPU)).toBe(true);
    expect(detectDedicated("Intel", "Intel(R) UHD Graphics 630", NO_WEBGPU)).toBe(false);
    expect(detectDedicated("Intel", "Intel(R) Iris(R) Xe Graphics", NO_WEBGPU)).toBe(false);
  });

  test("software and fallback adapters are never dedicated", () => {
    expect(detectDedicated("Software", "llvmpipe", NO_WEBGPU)).toBe(false);
    const fallback = {
      supported: true,
      adapterInfo: { vendor: "nvidia", isFallbackAdapter: true },
    };
    expect(detectDedicated("NVIDIA", "GeForce", fallback)).toBe(false);
  });
});

describe("releaseWebGLContext", () => {
  test("calls loseContext on the WEBGL_lose_context extension", () => {
    let lost = 0;
    const gl = fakeGl({ loseContext: () => lost++ });
    releaseWebGLContext(gl);
    expect(lost).toBe(1);
  });

  test("tolerates a missing extension or a throwing context", () => {
    expect(() => releaseWebGLContext(fakeGl(null))).not.toThrow();
    const throwing = {
      getExtension() {
        throw new Error("context lost");
      },
    } as unknown as WebGLRenderingContext;
    expect(() => releaseWebGLContext(throwing)).not.toThrow();
  });
});

describe("tryWebGL", () => {
  const original = (globalThis as { document?: unknown }).document;
  afterEach(() => {
    (globalThis as { document?: unknown }).document = original;
  });

  test("reports unsupported outside a browser", () => {
    (globalThis as { document?: unknown }).document = undefined;
    expect(tryWebGL("webgl2").supported).toBe(false);
  });

  test("reports unsupported when context creation returns null, without leaking", () => {
    installDocument(() => null);
    const caps = tryWebGL("webgl2");
    expect(caps.supported).toBe(false);
    expect(caps.reason).toContain("webgl2");
  });

  test("releases the context after every successful probe", () => {
    const losses: number[] = [];
    installDocument(() => fakeGl({ loseContext: () => losses.push(1) }, "NVIDIA", "RTX"));
    const first = tryWebGL("webgl2");
    const second = tryWebGL("webgl1");
    expect(first.supported).toBe(true);
    expect(first.unmaskedRenderer).toBe("RTX");
    expect(first.unmaskedVendor).toBe("NVIDIA");
    expect(second.supported).toBe(true);
    expect(losses).toHaveLength(2);
  });

  test("releases the context even when reading parameters throws", () => {
    let lost = 0;
    const gl = fakeGl({ loseContext: () => lost++ });
    (gl as unknown as { getParameter: () => never }).getParameter = () => {
      throw new Error("boom");
    };
    installDocument(() => gl);
    const caps = tryWebGL("webgl2");
    expect(caps.supported).toBe(false);
    expect(caps.reason).toBe("boom");
    expect(lost).toBe(1);
  });
});

function installDocument(getContext: () => unknown): void {
  (globalThis as { document?: unknown }).document = {
    createElement: () => ({ getContext }),
  };
}

function fakeGl(
  lose: { loseContext: () => void } | null,
  unmaskedVendor = "",
  unmaskedRenderer = "",
): WebGLRenderingContext {
  const debugInfo = { UNMASKED_VENDOR_WEBGL: 1, UNMASKED_RENDERER_WEBGL: 2 };
  return {
    MAX_TEXTURE_SIZE: 10,
    MAX_VERTEX_ATTRIBS: 11,
    getExtension(name: string) {
      if (name === "WEBGL_lose_context") return lose;
      if (name === "WEBGL_debug_renderer_info") return unmaskedVendor ? debugInfo : null;
      return null;
    },
    getParameter(param: number) {
      if (param === 1) return unmaskedVendor;
      if (param === 2) return unmaskedRenderer;
      return 4096;
    },
    getSupportedExtensions: () => ["OES_texture_float"],
  } as unknown as WebGLRenderingContext;
}
