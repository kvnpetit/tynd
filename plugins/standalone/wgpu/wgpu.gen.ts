export type AdapterInfo = {
  vendor: string;
  device: string;
  driver: string;
};

export type AvailabilityInfo = {
  available: boolean;
  backends: string[];
  reason: string | null;
};

export type AvailableArgs = null;

// `Auto` lets wgpu pick the platform default (DX12 / Metal / Vulkan / GLES).
export type Backend = "auto" | "vulkan" | "metal" | "dx12" | "gl";

export type CloseArgs = {
  id: number;
};

export type CloseResult = {
  closed: boolean;
};

export type CreateSurfaceArgs = {
  opts: SurfaceOpts | null;
};

export type ListArgs = null;

// Maps onto `wgpu::PowerPreference` 1:1.
export type PowerPreference = "high-performance" | "low-power";

/**
 *  `Fifo` (vsync, default). `Mailbox` reduces latency on supporting
 *  platforms; `Immediate` disables vsync (tear-prone, benchmark-only).
 */
export type PresentMode = "fifo" | "mailbox" | "immediate";

export type ResizeArgs = {
  id: number;
  width: number;
  height: number;
};

// `Bgra8unorm` is the most universally supported swap-chain format.
export type SurfaceFormat = "rgba8unorm" | "bgra8unorm" | "rgba16float";

/**
 *  `id` rendered as JS `number` to match the registry key the JS side
 *  uses for `resize` / `close` / event subscription.
 */
export type SurfaceHandle = {
  id: number;
  width: number;
  height: number;
  backend: string;
  adapter: AdapterInfo;
};

// Every field optional with sensible defaults.
export type SurfaceOpts = {
  title: string | null;
  width: number;
  height: number;
  backend: Backend;
  powerPreference: PowerPreference;
  presentMode: PresentMode;
  format: SurfaceFormat;
};

// === Result aliases (auto from #[command] return types) ===
export type AvailableResult = AvailabilityInfo;
export type CreateSurfaceResult = SurfaceHandle;
export type ResizeResult = null;
export type ListResult = SurfaceHandle[];

// === Command catalog (typed client catalog) ===
export const _commands = {
  available: {
    args: undefined as unknown as AvailableArgs,
    result: undefined as unknown as AvailableResult,
  },
  createSurface: {
    args: undefined as unknown as CreateSurfaceArgs,
    result: undefined as unknown as CreateSurfaceResult,
  },
  resize: {
    args: undefined as unknown as ResizeArgs,
    result: undefined as unknown as ResizeResult,
  },
  close: { args: undefined as unknown as CloseArgs, result: undefined as unknown as CloseResult },
  list: { args: undefined as unknown as ListArgs, result: undefined as unknown as ListResult },
} as const;
export type CommandsCatalog = typeof _commands;
