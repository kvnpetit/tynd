/**
 * `@tynd/plugin-wgpu` — single-import entry. Native OS windows backed
 * by Metal/DX12/Vulkan/GL, bypassing the webview compositor.
 * Without `--features native`, `createSurface()` rejects + `available()`
 * returns `{ available: false }`.
 */

import {
  composePluginClient,
  createHandleEvents,
  createResourceCloser,
  defineTypedPlugin,
} from "@tynd/plugin-sdk";
import type {
  AdapterInfo,
  AvailabilityInfo,
  Backend,
  CommandsCatalog,
  PowerPreference,
  PresentMode,
  SurfaceHandle as RawSurfaceHandle,
  SurfaceFormat,
  SurfaceOpts,
} from "./wgpu.gen";

export type {
  AdapterInfo,
  AvailabilityInfo,
  Backend,
  PowerPreference,
  PresentMode,
  RawSurfaceHandle,
  SurfaceFormat,
  SurfaceOpts,
};

// Mirrors `#[serde(default)]` on the Rust `SurfaceOpts` so callers may
// pass an empty object. specta-typescript@0.0.11 cannot emit `?:` for
// fields carrying serde defaults (unified mode), so this wrapper fills
// them client-side. Same pattern as `screen-rec`'s `RecordingOptions`.
export interface SurfaceOptions {
  title?: string | null;
  width?: number;
  height?: number;
  backend?: Backend;
  powerPreference?: PowerPreference;
  presentMode?: PresentMode;
  format?: SurfaceFormat;
}

function fillSurfaceDefaults(opts: SurfaceOptions): SurfaceOpts {
  return {
    title: opts.title ?? null,
    width: opts.width ?? 1280,
    height: opts.height ?? 720,
    backend: opts.backend ?? "auto",
    powerPreference: opts.powerPreference ?? "high-performance",
    presentMode: opts.presentMode ?? "fifo",
    format: opts.format ?? "bgra8unorm",
  };
}

const _api = defineTypedPlugin<CommandsCatalog>("wgpu");

/** Handle to a native wgpu surface (window) bound to a backend adapter. */
export interface WgpuSurface {
  readonly id: number;
  readonly width: number;
  readonly height: number;
  readonly backend: string;
  readonly adapter: AdapterInfo;
  resize(width: number, height: number): Promise<void>;
  close(): Promise<void>;
  onFrame(callback: (frame: number) => void): { dispose(): void };
  onResize(callback: (w: number, h: number) => void): { dispose(): void };
  onClose(callback: () => void): { dispose(): void };
  /** Native render / device errors (`wgpu:error`); repeated identical errors are throttled. */
  onError(callback: (message: string) => void): { dispose(): void };
}

/** Live wrappers by surface id so repeated `list()` calls do not stack listener sets. */
const surfaces = new Map<number, WgpuSurface>();

function wrap(raw: RawSurfaceHandle): WgpuSurface {
  const surface = buildSurface(raw);
  surfaces.set(raw.id, surface);
  return surface;
}

function buildSurface(raw: RawSurfaceHandle): WgpuSurface {
  let width = raw.width;
  let height = raw.height;
  const events = createHandleEvents(raw.id, "wgpu");
  events.disposeOn("wgpu:close", "id");
  const close = createResourceCloser(
    async () => {
      await _api.close({ id: raw.id });
    },
    () => {
      events.disposeAll();
      surfaces.delete(raw.id);
    },
  );
  return {
    get id() {
      return raw.id;
    },
    get width() {
      return width;
    },
    get height() {
      return height;
    },
    backend: raw.backend,
    adapter: raw.adapter,
    async resize(w, h) {
      await _api.resize({ id: raw.id, width: w, height: h });
      width = w;
      height = h;
    },
    close,
    onFrame(callback) {
      const off = events.on<{ frame: number }>("wgpu:frame", "id", (p) => callback(p.frame));
      return { dispose: off };
    },
    onResize(callback) {
      const off = events.on<{ width: number; height: number }>("wgpu:resize", "id", (p) => {
        width = p.width;
        height = p.height;
        callback(p.width, p.height);
      });
      return { dispose: off };
    },
    onClose(callback) {
      const off = events.on<unknown>("wgpu:close", "id", () => callback());
      return { dispose: off };
    },
    onError(callback) {
      const off = events.on<{ message?: unknown }>("wgpu:error", "id", (p) =>
        callback(typeof p.message === "string" ? p.message : "unknown wgpu error"),
      );
      return { dispose: off };
    },
  };
}

/** Frontend bindings for the `@tynd/plugin-wgpu` plugin (native windows + GPU adapters). */
export const wgpu = composePluginClient(_api, {
  /** Probe whether the host build supports native wgpu surfaces. */
  available(): Promise<AvailabilityInfo> {
    return _api.available() as Promise<unknown> as Promise<AvailabilityInfo>;
  },
  /** Open a native wgpu surface and return a handle bound to it. */
  async createSurface(opts?: SurfaceOptions): Promise<WgpuSurface> {
    const raw = (await _api.createSurface({
      opts: fillSurfaceDefaults(opts ?? {}),
    })) as unknown as RawSurfaceHandle;
    return wrap(raw);
  },
  /** List every still-open wgpu surface for this app. */
  async list(): Promise<WgpuSurface[]> {
    const raws = (await _api.list()) as RawSurfaceHandle[];
    const live = new Set(raws.map((raw) => raw.id));
    // Forget surfaces closed natively (their listeners are already disposed).
    for (const id of [...surfaces.keys()]) if (!live.has(id)) surfaces.delete(id);
    // Reuse cached wrappers: wrapping again would stack a new listener set.
    return raws.map((raw) => surfaces.get(raw.id) ?? wrap(raw));
  },
});

export default wgpu;
