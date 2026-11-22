# @tynd/plugin-wgpu

Native [`wgpu`](https://github.com/gfx-rs/wgpu) surface plugin for Tynd
apps. Spins up real OS windows backed by Metal / D3D12 / Vulkan / GL
that **bypass the Chromium / WKWebView renderer entirely** — for apps
that need every microsecond on the GPU:

| Use case                  | Why bypass the webview                                  |
|---------------------------|---------------------------------------------------------|
| CAD viewers               | 5–15% perf gain, no Chromium compositor tax             |
| Game engines              | Multi-window with shared `wgpu::Device` (no copy)       |
| Scientific visualisation  | Compute shaders without the browser sandbox limits      |
| Motion graphics tools     | Sub-frame latency for direct-manipulation editing       |
| ML / GPGPU                | Direct Rust → GPU buffer access, zero JS round-trip     |

> **Status: MVP scaffold.** The window-lifecycle surface is real
> (`createSurface`, `resize`, `close`, `list`, event bus). The
> render-command bridge that translates `THREE.WebGPURenderer` /
> `BABYLON.WebGPUEngine` calls into native wgpu encoder calls is
> **stubbed** — see [Status](#status) and [Deferred work](#deferred-work)
> below for what works today vs. what's still on the roadmap.

## Status

What ships in the MVP — what you can rely on today, and what is still a
deterministic stub.

| Surface area                                     | Status      |
|--------------------------------------------------|-------------|
| `wgpu.available()` — capability probe            | **works**   |
| `wgpu.createSurface()` — opens a real OS window  | **works**   |
| `WgpuSurface.resize(w, h)` — reconfigure swap    | **works**   |
| `WgpuSurface.close()` — tear down + free         | **works**   |
| `WgpuSurface.onFrame / onResize / onClose`       | **works**   |
| `wgpu.list()` — registry snapshot                | **works**   |
| Adapter info (vendor / device / driver)          | **works**   |
| Per-frame clear pass (background colour)         | **works**   |
| `WgpuSurface.submit(commands)` — render commands | **stub**    |
| Three.js / Babylon `render()` → native           | **stub**    |
| `wgpu.createCompute()` — headless GPU device     | not present |
| macOS / Linux `createSurface` (main-thread req.) | rejected    |

The **stub** rows return a deterministic
`"render-command bridge is stubbed in MVP"` error from the Rust side;
they exist on the JS surface so consumers can write code against the
final API today without source changes when the bridge lands.

**Requires** a Rust toolchain on the development machine; Tynd compiles the
plugin crate into the app's host automatically.

## Build flavours

| Cargo feature   | Native deps             | Behaviour                                                                             |
|-----------------|-------------------------|---------------------------------------------------------------------------------------|
| (default)       | none                    | Stub — every method that needs the GPU returns `"rebuild with --features native"`     |
| `native`        | `wgpu` + `tao` + ~10 MB | Full renderer; opens real OS windows on Metal / DX12 / Vulkan / GL                    |

Tynd enables the package's baseline `native` feature automatically. It remains
feature-gated for direct Cargo consumers because:

- The dep tree is heavy (~5–10 MB after release linking).
- Cold builds take 5–10 minutes (wgpu has a lot of generated SPIR-V tables).
- Linux + Windows need OS dev headers (Vulkan SDK, Win10 SDK).

Direct Cargo consumers that omit the feature still get the typed surface and
deterministic capability probe, but cannot open windows.

## Install

```sh
tynd add wgpu
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-wgpu"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "Editor" } })
```

No extra app configuration is required for the native renderer; listing
`"@tynd/plugin-wgpu"` compiles it into the app's host.

## Use

```ts
import { wgpu } from "@tynd/plugin-wgpu"

// 1. Capability probe — never throws.
const probe = await wgpu.available()
if (!probe.available) {
  console.warn("native wgpu unavailable:", probe.reason)
  // Fall back to a webview <canvas> renderer here.
  return
}

// 2. Open a native window.
const surface = await wgpu.createSurface({
  title: "CAD viewer",
  width: 1920,
  height: 1080,
  backend: "auto",                  // metal / dx12 / vulkan / gl
  powerPreference: "high-performance",
  presentMode: "fifo",              // or "mailbox" / "immediate"
  format: "bgra8unorm",
})

console.log(`backend=${surface.backend} adapter=${surface.adapter.device}`)

// 3. Drive an animation loop.
const offFrame = surface.onFrame((frame) => {
  // Push a per-frame command batch (stubbed in MVP — see below).
  // surface.submit([{ type: "clear", color: [0.012, 0.012, 0.02, 1.0] }]);
})

const offResize = surface.onResize((w, h) => {
  console.log(`window resized to ${w}×${h}`)
})

surface.onClose(() => {
  console.log("user closed the native window")
  offFrame.dispose()
  offResize.dispose()
})

// 4. Tear down explicitly when your panel unmounts.
await surface.close()
```

## Three.js adapter

A `THREE.WebGPURenderer`-shaped wrapper ships under
`@tynd/plugin-wgpu/three-adapter`:

```ts
import { wgpu } from "@tynd/plugin-wgpu"
import { ThreeNativeRenderer } from "@tynd/plugin-wgpu/three-adapter"
import * as THREE from "three"

const surface = await wgpu.createSurface({ width: 1920, height: 1080 })
const renderer = new ThreeNativeRenderer(surface)

const scene = new THREE.Scene()
const camera = new THREE.PerspectiveCamera(75, 16 / 9, 0.1, 1000)
camera.position.z = 5

renderer.onResize((w, h) => {
  camera.aspect = w / h
  camera.updateProjectionMatrix()
})

surface.onFrame(() => {
  renderer.render(scene, camera)
})
```

The renderer's lifecycle is real (auto-dispose on window close,
`setSize` → `surface.resize`, etc.); `render()` itself routes through
the **stubbed** IPC bridge today (see below). The public TypeScript
surface stays stable when the bridge lands — no consumer source changes
needed to upgrade.

## API reference

### `wgpu.available()`

```ts
type AvailabilityInfo = {
  available: boolean;
  backends: string[];   // e.g. ["metal", "vulkan", "gl"]
  reason?: string;      // why available is false, when known
};
```

Capability probe. Returns `available: false` when the plugin was built
without the `native` Cargo feature, **or** when no working wgpu adapter
is reachable (headless CI, broken driver). Never rejects.

### `wgpu.createSurface(opts?)`

```ts
type SurfaceOptions = {
  title?: string;            // default "Tynd wgpu surface"
  width?: number;            // default 1280 (logical px)
  height?: number;           // default 720
  backend?: "auto" | "vulkan" | "metal" | "dx12" | "gl";
  powerPreference?: "high-performance" | "low-power";
  presentMode?: "fifo" | "mailbox" | "immediate";
  format?: "rgba8unorm" | "bgra8unorm" | "rgba16float";
};
```

Opens a native window backed by a wgpu surface. Returns a
`WgpuSurface` object with `id`, `width`, `height`, `backend`, `adapter`
(vendor / device / driver) and the methods below.

### `WgpuSurface`

| Method                          | Behaviour                                                    |
|---------------------------------|--------------------------------------------------------------|
| `resize(w, h): Promise<void>`   | Reconfigure the swap chain                                   |
| `close(): Promise<void>`        | Close the window + free the surface                          |
| `onFrame(cb): Disposable`       | Per-frame tick — `cb(frameCount)`                            |
| `onResize(cb): Disposable`      | Native resize event — `cb(width, height)`                    |
| `onClose(cb): Disposable`       | Native close button event                                    |
| `submit(commands): Promise<void>` | **Stubbed in MVP** — see [Deferred work](#deferred-work)   |

### `wgpu.list()`

Snapshot of every active surface. Useful for inspector / debug panels.

## Deferred work

The MVP ships the surface lifecycle. Three pieces are explicitly **not
done** — each has a clear path forward:

### 1. `PluginCx::with_event_loop_proxy` accessor

`tao::Window` requires its `EventLoop` to live on the process **main
thread**. The Tynd host owns that thread for its own primary window and
the plugin SDK does not yet expose a handle on it.

The MVP works around this by spawning a dedicated event-loop thread per
`createSurface` call (`EventLoopBuilder::with_any_thread(true)`):

| Platform        | Status                                                              |
|-----------------|---------------------------------------------------------------------|
| Windows         | Works — winit's Win32 backend accepts non-main-thread loops          |
| Linux           | **Rejected** — tao is GTK there, which is not thread-safe and cannot run a second loop next to the host's own |
| macOS           | **Rejected** — Cocoa's run loop hard-requires the main thread       |

On the rejected platforms `createSurface` fails with an explanatory error and
`available()` returns `{ available: false, reason }`, so callers can fall back
to a webview canvas.

The proper fix is a new `PluginCx::with_event_loop_proxy()` accessor on
the SDK side that hands plugins an `EventLoopProxy<UserEvent>` plumbed
by the host's window setup. Once it lands, this plugin shrinks `window.rs`
by ~50 LoC and gains macOS and Linux support out of the box.

### 2. JSON render-command bridge

`bridge.rs::RenderCommand` sketches the JSON envelope but only the
`Clear` variant is decoded today. The full plan, in priority order:

1. **Clear-only** (`{ type: "clear", color: [r, g, b, a] }`) — lets a JS
   animation loop drive the bg colour from a sine wave. ~50 LoC.
2. **Buffer / pipeline upload + draw** — maps onto the WebGPU JS API
   1:1. A custom WGSL renderer on the JS side runs unchanged. ~600 LoC.
3. **Three.js / Babylon adapter shim** — `three-adapter.ts` intercepts
   the `WebGPURenderer` device + context creation and routes every
   encoder call through this bridge. Trickiest piece: bind-group /
   texture-view cache invariants the engines rely on.

The JS side batches one frame's worth of commands into a single IPC
round-trip — without batching the per-frame cost of N×IPC dominates and
erases the perf gain over the webview.

### 3. Compute-only headless mode

A future `wgpu.createCompute()` opens a `wgpu::Device` without a `tao`
window — useful for ML inference / scientific compute that doesn't
need to display anything. Same dispatch surface, same registry, no
event loop. Not in the MVP because the use cases are well-served by
[`@tynd/plugin-llm`](../llm) (LLMs) and direct WebGPU
(`@tynd/plugin-gpu` benchmark path) for everything else.

## Comparison vs. webview WebGPU

| Aspect                   | Webview WebGPU              | `@tynd/plugin-wgpu` (native)             |
|--------------------------|-----------------------------|------------------------------------------|
| Frame budget overhead    | 1–3 ms (Chromium compositor)| ~50 µs (direct surface present)          |
| Memory                   | shared with renderer process| dedicated wgpu allocator                 |
| Compute workgroup limits | sandbox-capped              | full driver limits                       |
| Multi-window             | one canvas per webview      | many surfaces, optionally shared device  |
| Tooling                  | RenderDoc / browser devtools| RenderDoc / Xcode / RGP / Nsight         |
| Three.js / Babylon       | drop-in (browser path)      | needs the adapter (today: stubbed)       |
| Build complexity         | none                        | `--features native` + system GPU SDK     |

Use the webview path when:
- You don't need >120 FPS at 4K.
- The 1–3 ms compositor budget is fine.
- You'd rather not ship the 5–10 MB native dep tree.

Use this plugin when:
- The compositor tax shows up in your frame trace.
- You need compute shaders without sandbox limits.
- You want multi-window with shared GPU resources.

## Build

```sh
# Default (no GPU deps) — fast.
cd plugins/wgpu/native && cargo build

# Real renderer — first build is 5–10 minutes from cold caches.
cd plugins/wgpu/native && cargo build --features native
```

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
