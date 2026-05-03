/**
 * `@tynd/core/config` — V1 config loader (c12-backed).
 *
 * Loads `tynd.config.{ts,js,mjs,json}` with layered overrides:
 *  - Base file
 *  - Env-specific overrides (`tynd.config.development.ts`, `tynd.config.production.ts`)
 *  - Local overrides (`tynd.config.local.ts`, gitignored)
 *  - `extends:` chains — pull templates from GitHub / GitLab / npm / file paths
 *
 * Built on `c12` — same loader Nuxt and Nitro use. `defu` deep-merge is
 * automatic. `watchConfig` powers HMR of config edits in `tynd dev`.
 *
 * @example
 *   import { loadTyndConfig } from "@tynd/core/config"
 *
 *   const { config, configFile, layers } = await loadTyndConfig({
 *     cwd: process.cwd(),
 *     defaults: { runtime: "full", window: { width: 1200, height: 800 } },
 *   })
 */

import { loadConfig, type ResolvedConfig, type WatchConfigOptions, watchConfig } from "c12";

/**
 * Resolved Tynd app config — a JSON-serialisable shape. The CLI carries
 * a richer schema with valibot-validated fields; this one stays minimal
 * so `@tynd/core` doesn't take a build-time dep on the CLI.
 */
export interface TyndAppConfig {
  runtime: "full" | "lite" | "full-custom" | "lite-custom";
  backend: string;
  frontendDir?: string;
  frontendEntry?: string;
  devUrl?: string;
  devCommand?: string;
  icon?: string;
  binaryArgs?: string[];
  /**
   * U1 Layer 1 — Bun↔host IPC wire format. Default `"json"` (legacy
   * newline-delimited JSON, back-compat). `"cbor"` enables length-prefixed
   * CBOR frames — ~10× decode throughput on bursty/large payloads, but
   * requires the `binary-ipc` Cargo feature on the host runtime crate
   * (`tynd-full`). The CLI translates this into `TYND_IPC_FORMAT=cbor`
   * on the spawned Bun child env so both sides agree before the first
   * frame.
   *
   * When set to `"cbor"` against a runtime built without `binary-ipc`,
   * the host quietly falls back to JSON to avoid deadlocking the IPC
   * channel. The Bun side does the same — both check the env var
   * independently.
   */
  ipcFormat?: "json" | "cbor";
  window?: {
    title?: string;
    width?: number;
    height?: number;
    center?: boolean;
    devtools?: boolean;
  };
  plugins?: Array<string | { pkg: string; apis?: string[] }>;
  [key: string]: unknown;
}

/**
 * Options accepted by `loadTyndConfig`. Mirrors a useful subset of c12's
 * options — restrict here to the surface the framework actually exposes.
 */
export interface LoadTyndConfigOptions {
  /** Project root (defaults to `process.cwd()`). */
  cwd?: string;
  /** Defaults merged underneath the user config (lowest layer). */
  defaults?: Partial<TyndAppConfig>;
  /** Overrides applied above all layers (highest layer). */
  overrides?: Partial<TyndAppConfig>;
  /** Pass `true` to require a config file — throws if absent. */
  required?: boolean;
}

/**
 * Resolved config + provenance. `configFile` is `null` when only
 * defaults / overrides applied (no file on disk).
 */
export interface ResolvedTyndConfig {
  config: TyndAppConfig;
  configFile: string | null;
  layers: ResolvedConfig<TyndAppConfig>["layers"];
}

const CONFIG_NAME = "tynd";

/**
 * Resolve and merge `tynd.config.*` for a project root.
 *
 * Layers (lowest → highest precedence):
 *  1. `defaults`
 *  2. `extends:` chain (if any)
 *  3. `tynd.config.{ts,js,mjs,json}`
 *  4. `tynd.config.<env>.{ts,js,mjs,json}`  (env = `NODE_ENV`)
 *  5. `tynd.config.local.{ts,js,mjs,json}`  (gitignored, dev-only)
 *  6. `overrides`
 */
export async function loadTyndConfig(
  opts: LoadTyndConfigOptions = {},
): Promise<ResolvedTyndConfig> {
  const resolved = await loadConfig<TyndAppConfig>({
    name: CONFIG_NAME,
    cwd: opts.cwd,
    defaults: opts.defaults as TyndAppConfig | undefined,
    overrides: opts.overrides as TyndAppConfig | undefined,
    dotenv: true,
    envName: process.env.NODE_ENV ?? "development",
  });
  if (opts.required && !resolved.configFile) {
    throw new Error(
      "tynd.config.{ts,js,mjs,json} not found. Run `bunx @tynd/cli init` to scaffold one.",
    );
  }
  return {
    config: resolved.config as TyndAppConfig,
    configFile: resolved.configFile ?? null,
    layers: resolved.layers ?? [],
  };
}

/**
 * Watch `tynd.config.*` for edits and re-run a callback with the
 * resolved config. Used by `tynd dev` to HMR config changes without
 * restarting the host process.
 *
 * Returns an `unwatch` function — call it to stop the watcher.
 */
export async function watchTyndConfig(
  opts: LoadTyndConfigOptions & {
    onUpdate: (config: TyndAppConfig) => void | Promise<void>;
  },
): Promise<() => Promise<void>> {
  const watcher = await watchConfig<TyndAppConfig>({
    name: CONFIG_NAME,
    cwd: opts.cwd,
    defaults: opts.defaults as TyndAppConfig | undefined,
    overrides: opts.overrides as TyndAppConfig | undefined,
    dotenv: true,
    envName: process.env.NODE_ENV ?? "development",
    onUpdate: ({ newConfig }) => opts.onUpdate(newConfig.config as TyndAppConfig),
  } satisfies WatchConfigOptions<TyndAppConfig>);
  return () => watcher.unwatch();
}
