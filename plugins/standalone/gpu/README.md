# @tynd/plugin-gpu

GPU diagnostics, readiness checks and a quick benchmark for Tynd apps —
so the frontend can pick a quality preset, refuse to spin up a heavy
3D pipeline on a software adapter, or fall back gracefully on
integrated / mobile GPUs.

Pure JavaScript, zero native dependencies. Works with every Tynd
Bun runtime.

## Install

```sh
tynd add gpu
```

## Use

```ts
import { gpu } from "@tynd/plugin-gpu"

// One-shot capability snapshot — cached after first call.
const info = await gpu.info()
console.log(info.vendor, info.renderer, info.isDedicated)
// → "NVIDIA"  "NVIDIA GeForce RTX 4070"  true

if (!info.webgpu.supported) {
  console.warn("WebGPU disabled:", info.webgpu.reason)
}

// Hard fail when WebGPU is mandatory — error includes the diagnostic
// envelope (vendor, renderer, reason).
await gpu.requireWebGPU()

// Pick a quality preset based on a quick benchmark.
const bench = await gpu.benchmark()
const preset =
  bench.category === "extreme" ? "ultra" :
  bench.category === "high"    ? "high"  :
  bench.category === "medium"  ? "medium" : "low"

// Specific feature gate.
if (await gpu.supportsFeature("shader-f16")) {
  enableHalfPrecisionPipeline()
}

// Best-effort memory pressure (Chrome / Edge only — `available: false`
// elsewhere).
const mem = await gpu.memoryStatus()
if (mem.available && mem.usedMB && mem.usedMB > 800) {
  reduceTextureQuality()
}
```

## API

| Export | Purpose |
|---|---|
| `gpu.info()` | Full capability snapshot (vendor, renderer, dedicated, WebGL1/WebGL2/WebGPU caps). Cached. |
| `gpu.requireWebGPU()` | Throw with a diagnostic message if WebGPU is unavailable. |
| `gpu.requireWebGL2()` | Same, for WebGL 2. |
| `gpu.benchmark()` | Quick GPU benchmark (~500 ms). WebGPU compute matmul, WebGL fragment-load fallback. |
| `gpu.supportsFeature(name)` | WebGPU adapter feature flag lookup. |
| `gpu.memoryStatus()` | Best-effort memory pressure snapshot. |

### `GpuInfo`

| Field | Notes |
|---|---|
| `vendor` | `"NVIDIA"` / `"AMD"` / `"Intel"` / `"Apple"` / `"Qualcomm"` / `"ARM"` / `"Imagination"` / `"Software"` / raw string. |
| `renderer` | Marketing string (`"NVIDIA GeForce RTX 4070"`, `"Apple M3 Pro"`). |
| `memoryMB` | System RAM hint via `navigator.deviceMemory` (rounded, GB → MB). VRAM is not exposed by any browser. |
| `isDedicated` | Heuristic. NVIDIA + Apple silicon = `true`, AMD discrete = `true`, discrete Intel Arc = `true`, AMD APUs / integrated Intel / Mali / Adreno = `false`. WebGPU's `isFallbackAdapter` overrides. |
| `webgl1` / `webgl2` | `{ supported, maxTextureSize, maxVertexAttribs, extensions, unmaskedVendor, unmaskedRenderer }`. |
| `webgpu` | `{ supported, adapterInfo: { vendor, architecture, device, description, isFallbackAdapter }, features, limits }`. |

### `BenchmarkResult`

| Field | Notes |
|---|---|
| `durationMs` | Wall-clock duration. |
| `opsPerSec` | Throughput (multiply-adds for WebGPU, fragments for WebGL). Not directly comparable across backends. |
| `score` | Normalised 0-100. ~30 = integrated / mobile, ~55 = Apple silicon, ~80 = mid-range discrete, ~95+ = high-end discrete. |
| `category` | `"low" \| "medium" \| "high" \| "extreme"`. Buckets at 25 / 50 / 80. |
| `backend` | `"webgpu"` (preferred) or `"webgl"` (fallback). |

## Why headless / pure-JS

Every signal lives in the webview already (`navigator.gpu`,
`WebGL_debug_renderer_info`, `navigator.deviceMemory`,
`performance.memory`). A native Rust crate would buy nothing and would
force the consumer onto `runtime: "*-custom"`. This plugin stays pure
JS so any Tynd app can pull it in without changing its build profile.

## Caveats

- **Renderer string masking.** Many user agents (notably Safari, and
  Firefox by default) strip `UNMASKED_RENDERER_WEBGL` for fingerprinting
  reasons. When that happens we fall back to the WebGPU adapter info if
  available, otherwise vendor/renderer come back as `"unknown"`.
- **VRAM.** No browser API exposes dedicated VRAM. `memoryMB` reflects
  system RAM via `navigator.deviceMemory` — useful for "is this device
  low-end overall" but not for "do I have enough VRAM for a 2k texture".
- **Benchmark drift.** The score buckets are calibrated against a small
  reference set. Treat them as a coarse signal for picking a preset, not
  a benchmark suite result.
- **Software adapters.** A SwiftShader / llvmpipe / "Microsoft Basic
  Render" renderer is reported with `vendor: "Software"` and
  `isDedicated: false`; the WebGPU fallback adapter sets
  `webgpu.adapterInfo.isFallbackAdapter = true`.

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
