import { runBenchmark } from "./benchmark";
import { releaseWebGLContext } from "./webgl";

export interface GpuInfo {
  vendor: string;
  renderer: string;
  driverVersion?: string;
  memoryMB?: number;
  isDedicated: boolean;

  webgl1: WebGLCaps;
  webgl2: WebGLCaps;
  webgpu: WebGPUCaps;
}

export interface WebGLCaps {
  supported: boolean;
  reason?: string;
  maxTextureSize?: number;
  maxVertexAttribs?: number;
  extensions?: readonly string[];
  unmaskedVendor?: string;
  unmaskedRenderer?: string;
}

export interface WebGPUCaps {
  supported: boolean;
  reason?: string;
  adapterInfo?: {
    vendor: string;
    architecture?: string;
    device?: string;
    description?: string;
    isFallbackAdapter?: boolean;
  };
  features?: readonly string[];
  limits?: Record<string, number>;
}

export interface BenchmarkResult {
  durationMs: number;
  opsPerSec: number;
  score: number;
  category: BenchmarkCategory;
  backend: "webgpu" | "webgl";
}

export type BenchmarkCategory = "low" | "medium" | "high" | "extreme";

export interface MemoryStatus {
  usedMB?: number;
  totalMB?: number;
  deviceMemoryMB?: number;
  available: boolean;
}

let cachedInfo: Promise<GpuInfo> | null = null;

export const gpu = {
  info(): Promise<GpuInfo> {
    cachedInfo ??= collectGpuInfo();
    return cachedInfo;
  },

  /** Re-probe adapters and contexts, replacing a result cached before browser GPU initialization. */
  refreshInfo(): Promise<GpuInfo> {
    cachedInfo = collectGpuInfo();
    return cachedInfo;
  },

  async requireWebGPU(): Promise<void> {
    const i = await gpu.info();
    if (!i.webgpu.supported) {
      throw new Error(
        `WebGPU not available: ${i.webgpu.reason ?? "unknown reason"} (vendor=${i.vendor || "?"}, renderer=${i.renderer || "?"})`,
      );
    }
  },

  async requireWebGL2(): Promise<void> {
    const i = await gpu.info();
    if (!i.webgl2.supported) {
      throw new Error(
        `WebGL2 not available: ${i.webgl2.reason ?? "context creation failed"} (vendor=${i.vendor || "?"}, renderer=${i.renderer || "?"})`,
      );
    }
  },

  benchmark(): Promise<BenchmarkResult> {
    return runBenchmark();
  },

  async supportsFeature(name: string): Promise<boolean> {
    const i = await gpu.info();
    return i.webgpu.features?.includes(name) ?? false;
  },

  async memoryStatus(): Promise<MemoryStatus> {
    const result: MemoryStatus = { available: false };

    const deviceMemoryMB = readDeviceMemoryMB();
    if (deviceMemoryMB !== undefined) {
      result.deviceMemoryMB = deviceMemoryMB;
      result.available = true;
    }

    const perfMem =
      typeof performance === "undefined"
        ? undefined
        : (performance as Performance & { memory?: PerfMemory }).memory;
    if (perfMem && typeof perfMem.usedJSHeapSize === "number") {
      result.usedMB = Math.round(perfMem.usedJSHeapSize / (1024 * 1024));
      if (typeof perfMem.totalJSHeapSize === "number") {
        result.totalMB = Math.round(perfMem.totalJSHeapSize / (1024 * 1024));
      }
      result.available = true;
    }

    return result;
  },
};

interface PerfMemory {
  usedJSHeapSize?: number;
  totalJSHeapSize?: number;
  jsHeapSizeLimit?: number;
}

function readDeviceMemoryMB(): number | undefined {
  if (typeof navigator === "undefined") return;
  const nav = navigator as Navigator & { deviceMemory?: number };
  return typeof nav.deviceMemory === "number" ? nav.deviceMemory * 1024 : undefined;
}

async function collectGpuInfo(): Promise<GpuInfo> {
  const [webgpuCaps, webgl2Caps, webgl1Caps] = await Promise.all([
    tryWebGPUAdapter(),
    Promise.resolve(tryWebGL("webgl2")),
    Promise.resolve(tryWebGL("webgl1")),
  ]);

  const adapterRenderer =
    webgpuCaps.adapterInfo?.description ||
    webgpuCaps.adapterInfo?.device ||
    webgpuCaps.adapterInfo?.architecture ||
    "";
  const renderer =
    adapterRenderer || webgl2Caps.unmaskedRenderer || webgl1Caps.unmaskedRenderer || "";
  const vendorRaw =
    webgpuCaps.adapterInfo?.vendor || webgl2Caps.unmaskedVendor || webgl1Caps.unmaskedVendor || "";

  const vendor = parseVendor(vendorRaw, renderer);
  const isDedicated = detectDedicated(vendor, renderer, webgpuCaps);

  const memoryMB = readDeviceMemoryMB();

  return {
    vendor,
    renderer: renderer || "unknown",
    memoryMB,
    isDedicated,
    webgl1: webgl1Caps,
    webgl2: webgl2Caps,
    webgpu: webgpuCaps,
  };
}

async function tryWebGPUAdapter(): Promise<WebGPUCaps> {
  if (typeof navigator === "undefined" || !("gpu" in navigator) || !navigator.gpu) {
    return {
      supported: false,
      reason: "navigator.gpu not exposed (browser/webview too old or WebGPU disabled)",
    };
  }
  try {
    const adapter = await navigator.gpu.requestAdapter({
      powerPreference: "high-performance",
    });
    if (!adapter) {
      return { supported: false, reason: "navigator.gpu.requestAdapter returned null" };
    }
    return {
      supported: true,
      adapterInfo: extractAdapterInfo(adapter),
      features: [...adapter.features].sort(),
      limits: extractLimits(adapter.limits),
    };
  } catch (e) {
    return { supported: false, reason: stringifyError(e) };
  }
}

function extractAdapterInfo(adapter: GPUAdapter): WebGPUCaps["adapterInfo"] {
  const info = (adapter as GPUAdapter & { info?: GPUAdapterInfo }).info;
  if (!info) return;
  return {
    vendor: info.vendor || "",
    architecture: info.architecture || undefined,
    device: info.device || undefined,
    description: info.description || undefined,
    isFallbackAdapter: info.isFallbackAdapter,
  };
}

function extractLimits(limits: GPUSupportedLimits): Record<string, number> {
  const out: Record<string, number> = {};
  for (const key in limits) {
    const value = (limits as unknown as Record<string, unknown>)[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      out[key] = value;
    }
  }
  return out;
}

/** @internal Exported for tests; not part of the public API. */
export function tryWebGL(version: "webgl1" | "webgl2"): WebGLCaps {
  if (typeof document === "undefined") {
    return { supported: false, reason: "document not available (non-browser environment)" };
  }
  const contextId = version === "webgl2" ? "webgl2" : "webgl";
  let gl: WebGLRenderingContext | WebGL2RenderingContext | null;
  try {
    const canvas = document.createElement("canvas");
    gl = (
      version === "webgl2"
        ? canvas.getContext("webgl2")
        : (canvas.getContext("webgl") ?? canvas.getContext("experimental-webgl"))
    ) as WebGLRenderingContext | WebGL2RenderingContext | null;
  } catch (e) {
    return { supported: false, reason: stringifyError(e) };
  }
  if (!gl) {
    return { supported: false, reason: `${contextId} context creation returned null` };
  }

  try {
    const ext = gl.getExtension("WEBGL_debug_renderer_info") as {
      UNMASKED_VENDOR_WEBGL: number;
      UNMASKED_RENDERER_WEBGL: number;
    } | null;

    const caps: WebGLCaps = {
      supported: true,
      maxTextureSize: numberOr(gl.getParameter(gl.MAX_TEXTURE_SIZE)),
      maxVertexAttribs: numberOr(gl.getParameter(gl.MAX_VERTEX_ATTRIBS)),
      extensions: gl.getSupportedExtensions() ?? [],
    };

    if (ext) {
      const unmaskedVendor = gl.getParameter(ext.UNMASKED_VENDOR_WEBGL);
      const unmaskedRenderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL);
      if (typeof unmaskedVendor === "string") caps.unmaskedVendor = unmaskedVendor;
      if (typeof unmaskedRenderer === "string") caps.unmaskedRenderer = unmaskedRenderer;
    }

    return caps;
  } catch (e) {
    return { supported: false, reason: stringifyError(e) };
  } finally {
    // Each probe would otherwise leave a live context behind until the GC runs, and
    // refreshInfo()/benchmark loops can hit the browser's context cap.
    releaseWebGLContext(gl);
  }
}

function numberOr(v: unknown): number | undefined {
  return typeof v === "number" && Number.isFinite(v) ? v : undefined;
}

/** @internal Exported for tests; not part of the public API. */
export function parseVendor(rawVendor: string, renderer: string): string {
  const haystack = `${rawVendor} ${renderer}`.toLowerCase();
  if (/nvidia|geforce|quadro|tesla/.test(haystack)) return "NVIDIA";
  if (/amd|radeon|ryzen/.test(haystack)) return "AMD";
  if (/apple|m1|m2|m3|m4/.test(haystack)) return "Apple";
  if (/intel|iris|hd graphics|uhd graphics|\barc\b/.test(haystack)) return "Intel";
  if (/qualcomm|adreno/.test(haystack)) return "Qualcomm";
  if (/arm|mali/.test(haystack)) return "ARM";
  if (/imagination|powervr/.test(haystack)) return "Imagination";
  if (/microsoft basic render|swiftshader|llvmpipe|software/.test(haystack)) {
    return "Software";
  }
  return rawVendor || "unknown";
}

/** @internal Exported for tests; not part of the public API. */
export function detectDedicated(vendor: string, renderer: string, webgpu: WebGPUCaps): boolean {
  if (webgpu.adapterInfo?.isFallbackAdapter) return false;
  if (vendor === "Software") return false;
  if (vendor === "NVIDIA" || vendor === "Apple") return true;
  if (vendor === "AMD") {
    return !/radeon\(tm\) graphics$|vega \d+ graphics/i.test(renderer);
  }
  if (vendor === "Intel") {
    // Integrated Intel GPUs stay false; discrete Arc / Iris Xe MAX / Data Center parts are dedicated.
    // Integrated Arc ("Intel(R) Arc(TM) Graphics") has no model token; discrete Arc is "A770", "B580", "Pro A60".
    return /\barc(\(tm\))?\s+(pro\s+)?[abc]\d{2,3}\b|iris(\(r\))?\s+xe\s+max|data center gpu/i.test(
      renderer,
    );
  }
  return false;
}

function stringifyError(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return "unknown error";
  }
}
