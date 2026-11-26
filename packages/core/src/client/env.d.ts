/// <reference lib="dom" />

/**
 * Ambient types for globals + bundler env injected by the Tynd host /
 * CLI. Reference this file from your frontend `tsconfig.json` (added
 * automatically by `tynd init`):
 *   {
 *     "compilerOptions": {
 *       "types": ["@tynd/core/env"]
 *     }
 *   }
 * — or, equivalently, drop a triple-slash directive in `src/global.d.ts`:
 *   /// <reference types="@tynd/core/env" />
 * Without this file the typed shim members below exist only inside
 * `_internal.ts`; declaring them globally lets app code read the native
 * runtime metadata and the bundler-injected `import.meta.env` flags
 * without `as` casts.
 */

declare global {
  /**
   * Native bridge surface exposed by the host before any user JS runs.
   * Available in Tynd Bun host contexts.
   * Always check for presence
   * (`if (window.__tynd__)`) — this object is `undefined` in plain
   * browser, in test, in SSR, and during hot-module replacement before
   * the shim re-installs.
   */
  // Property names + modifiers (non-optional `__tynd__`, optional everything
  // else) intentionally match the in-source declarations in
  // `client/_internal.ts` and `client/window.ts` so TS can merge the
  // ambient augmentations without TS2687 ("identical modifiers"). The
  // tradeoff: app code that runs outside the host (test, SSR, plain
  // browser) should still guard `if (window.__tynd__)`. We document that
  // here rather than weakening the type.
  interface Window {
    __tynd__: {
      call: (fn: string, args: unknown[]) => unknown;
      os_call: (api: string, method: string, args: unknown) => Promise<unknown>;
      os_on: (name: string, handler: (data: unknown) => void) => () => void;
      on: (name: string, handler: (payload: unknown) => void) => () => void;
      off: (name: string, handler: (payload: unknown) => void) => void;
    };

    /** Bun is Tynd's only JavaScript runtime. */
    __TYND_RUNTIME__?: "bun";
    /** Enabled in dev (`tynd dev`); falsy in production builds. */
    __tynd_dev__?: boolean;
    /** Label of the current window (`"main"` for the primary window). */
    __TYND_WINDOW_LABEL__?: string;
    /** Current webview label. Equals window label for top-level webviews. */
    __TYND_WEBVIEW_LABEL__?: string;
  }

  /**
   * Frontend build-tool env (`DEV`/`PROD`/`MODE` mirror Vite's contract)
   * plus an optional application-defined version pin. App code can use these
   * without polluting `process.env` (which is empty in a webview).
   */
  interface ImportMeta {
    readonly env: {
      /** `true` when running under `tynd dev`. */
      readonly DEV: boolean;
      /** `true` when bundled by `tynd build` / `tynd start`. */
      readonly PROD: boolean;
      /** `"development" | "production"` — convenient for switch statements. */
      readonly MODE: "development" | "production";
      /** Version of `@tynd/core` the bundle was built against. */
      readonly TYND_VERSION?: string;
    };
  }
}

// `export {}` turns this from a script into a module. Without it, the
// `declare global` augmentation collides with the parent file's globals
// instead of merging into them.
export {};
