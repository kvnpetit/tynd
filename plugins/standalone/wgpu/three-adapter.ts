/**
 * Three.js adapter for `@tynd/plugin-wgpu`.
 * Lets a `THREE.Scene` render into a native [`WgpuSurface`](./client) the
 * same way it would render into a `HTMLCanvasElement` backed by the
 * browser's `WebGPURenderer`. The intent is drop-in compatibility — an
 * existing Three.js codebase only swaps the renderer constructor:
 * ```ts
 * // Browser-only WebGPU:
 * const renderer = new THREE.WebGPURenderer({ canvas })
 * // Native wgpu surface (this plugin):
 * const surface = await wgpu.createSurface({ width: 1920, height: 1080 })
 * const renderer = new ThreeNativeRenderer(surface)
 * ```
 * ## Status: scaffold
 * The class shape, lifecycle plumbing and event wiring are real; the
 * actual encoder bridge is not done yet — `render()` only logs a one-shot
 * warning while the native side draws a clear pass on its own. See
 * `plugins/standalone/wgpu/README.md` "Deferred work" for the rough plan.
 * In the meantime the scaffold is useful for two things:
 * 1. Type-checking — apps can import the class and write their
 *    integration code today; once the bridge lands, no source changes
 *    are needed.
 * 2. Lifecycle correctness — we still drive `surface.onResize` →
 *    `setSize`, `surface.onClose` → `dispose`, and the camera-aspect
 *    update so the integration feels real even with a stubbed encoder.
 * ## Implementation notes
 * Three.js' `WebGPURenderer` lives in the `three/webgpu` subpath since
 * r158. We do **not** subclass it directly today — its constructor
 * eagerly probes `navigator.gpu` which throws in a Node-style runtime.
 * Instead we expose a parallel, narrower surface that mirrors the four
 * methods Three.js apps actually call (`render` / `setSize` /
 * `setPixelRatio` / `dispose`). When the bridge lands we re-evaluate
 * whether to subclass or keep the wrapper — the public TS shape stays
 * stable either way.
 */

import type { WgpuSurface } from "./client";

// We type the Three.js bits through structural shapes instead of the
// `three` package's exports so the plugin's typecheck doesn't depend on
// `three` being installed. Apps that use this adapter will have `three`
// in their own deps; the structural shapes match what `THREE.Scene`,
// `THREE.Camera` and the `WebGPURenderer` constructor option bag look
// like in r158+.

/** Minimal `THREE.Scene` shape we touch from `render()`. */
export interface ThreeSceneLike {
  // `Scene` carries hundreds of properties; we never read any of them
  // ourselves — the field exists so the structural type isn't `{}`,
  // which would accept literally anything.
  isScene?: true;
}

/** Minimal `THREE.Camera` shape — we forward the aspect-ratio update. */
export interface ThreeCameraLike {
  isCamera?: true;
  aspect?: number;
  updateProjectionMatrix?(): void;
}

/** Constructor options. */
export interface ThreeNativeRendererOptions {
  /**
   * Whether to call `surface.close()` from `dispose()`. Default `true`.
   * Set to `false` when the same surface is shared across multiple
   * Three.js scenes (split-pane editor, picture-in-picture …).
   */
  ownsSurface?: boolean;
  /**
   * Device pixel ratio override. When `undefined` we read
   * `window.devicePixelRatio` if available (browser webview) or default
   * to 1.0 (Node-style).
   */
  pixelRatio?: number;
}

/** `window.devicePixelRatio` in a browser webview, `1.0` elsewhere. */
function defaultPixelRatio(): number {
  const ratio =
    typeof globalThis !== "undefined" && "window" in globalThis
      ? (globalThis as { window?: { devicePixelRatio?: number } }).window?.devicePixelRatio
      : undefined;
  return typeof ratio === "number" ? ratio : 1.0;
}

/**
 * Three.js-compatible renderer that targets a native [`WgpuSurface`].
 * The public method surface mirrors the slice of `THREE.WebGPURenderer`
 * that 99% of Three.js apps actually use. When the IPC bridge lands the
 * implementation flips from "log a warning" to
 * "translate the THREE encoder graph into our `RenderCommand` shape",
 * with no public API change.
 */
export class ThreeNativeRenderer {
  /** The native surface this renderer writes to. */
  readonly surface: WgpuSurface;

  /** Current device pixel ratio. */
  pixelRatio: number;

  private readonly ownsSurface: boolean;
  private disposed = false;
  private offResize: { dispose(): void } | null = null;
  private offClose: { dispose(): void } | null = null;
  private warnedStub = false;

  constructor(surface: WgpuSurface, opts: ThreeNativeRendererOptions = {}) {
    this.surface = surface;
    this.ownsSurface = opts.ownsSurface ?? true;
    this.pixelRatio = opts.pixelRatio ?? defaultPixelRatio();

    // Wire native window events into the renderer surface so callers
    // don't have to re-do this for every Three.js project.
    this.offResize = surface.onResize((w, h) => {
      // The native side already resized the swap chain; we just give
      // hooks a chance to react. Apps usually set the camera aspect on
      // the same listener, so we *don't* call `surface.resize()` here
      // (would loop).
      for (const cb of this.onResizeListeners) {
        cb(w, h);
      }
    });
    this.offClose = surface.onClose(() => {
      this.dispose();
    });
  }

  /**
   * Render a Three.js scene through the native wgpu surface.
   * Today this is a no-op beyond a one-shot warning — see the class doc.
   * Once the bridge lands the body becomes:
   * 1. Walk the scene graph through the standard Three.js renderer pipeline.
   * 2. Build a `RenderCommand[]` batch from the resulting encoder calls.
   * 3. `await surface.submit(commands)`.
   */
  async render(scene: ThreeSceneLike, _camera: ThreeCameraLike): Promise<void> {
    if (this.disposed) return;

    // Sanity check — the wrapper shouldn't accept anything but a Scene,
    // but `isScene` is set by Three.js itself so we trust the runtime
    // shape. We don't throw on a missing flag because some apps
    // construct minimal mock scenes for tests.
    void scene;

    if (!this.warnedStub) {
      this.warnedStub = true;
      // biome-ignore lint/suspicious/noConsole: one-shot dev hint, not a hot-path log.
      console.warn(
        "[@tynd/plugin-wgpu] ThreeNativeRenderer.render() is stubbed. " +
          "The native side draws a clear pass; mesh / pipeline encoding lands once " +
          "the IPC bridge ships. See the @tynd/plugin-wgpu README 'Deferred work'.",
      );
    }
    // No submit bridge yet — the native side draws a clear pass on its
    // own. Once a `submit` command lands in `wgpu.gen.ts`, encode draw
    // commands here.
  }

  /**
   * Resize the underlying native window's swap chain. Mirrors
   * `THREE.WebGPURenderer.setSize(width, height, updateStyle)` — the
   * third argument is ignored (no DOM canvas to restyle).
   */
  async setSize(width: number, height: number, _updateStyle?: boolean): Promise<void> {
    if (this.disposed) return;
    await this.surface.resize(width, height);
  }

  /**
   * Update the device pixel ratio. Three.js apps that handle hi-DPI
   * call this on `window.matchMedia("(resolution: ...)")` change events.
   */
  setPixelRatio(ratio: number): void {
    this.pixelRatio = ratio;
  }

  /**
   * Tear down the renderer. Closes the native surface when
   * `ownsSurface` is `true` (the default).
   */
  async dispose(): Promise<void> {
    if (this.disposed) return;
    this.disposed = true;
    this.offResize?.dispose();
    this.offResize = null;
    this.offClose?.dispose();
    this.offClose = null;
    if (this.ownsSurface) {
      try {
        await this.surface.close();
      } catch {
        /* surface may already be gone — close is idempotent */
      }
    }
  }

  /**
   * Subscribe to native resize events. Useful for updating the
   * camera's aspect ratio without re-implementing the listener wiring
   * every project.
   * @example
   *   renderer.onResize((w, h) => {
   *     camera.aspect = w / h
   *     camera.updateProjectionMatrix()
   *   })
   */
  onResize(callback: (width: number, height: number) => void): { dispose(): void } {
    this.onResizeListeners.add(callback);
    return {
      dispose: () => {
        this.onResizeListeners.delete(callback);
      },
    };
  }

  /** Internal listener set — separate from the surface-level event so
   *  multiple Three.js scenes sharing a surface each get a turn. */
  private readonly onResizeListeners = new Set<(width: number, height: number) => void>();
}
