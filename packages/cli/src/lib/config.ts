// The config types are generated from the Rust config model (crates/cli/src/config/model.rs);
// run `bun scripts/gen-api-catalog.ts` after changing it.
export type * from "./config/types.generated";

import type { TyndConfig } from "./config/types.generated";

export interface TyndConfigContext {
  /** Value selected through `tynd --env`, or `development` by default. */
  envName: string;
}

export type TyndConfigFactory<T extends TyndConfig = TyndConfig> = (
  context: TyndConfigContext,
) => T | Promise<T>;

export type TyndConfigExport<T extends TyndConfig = TyndConfig> = T | TyndConfigFactory<T>;

/** Preserve literal types while checking a static or environment-aware config. */
export function defineConfig<const T extends TyndConfig>(config: T): T;
export function defineConfig<const T extends TyndConfig>(
  config: TyndConfigFactory<T>,
): TyndConfigFactory<T>;
export function defineConfig(config: TyndConfigExport): TyndConfigExport {
  return config;
}
