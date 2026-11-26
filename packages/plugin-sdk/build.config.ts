import { defineBuildConfig } from "unbuild";

/**
 * unbuild config for `@tynd/plugin-sdk`.
 * unbuild emits `dist/index.mjs` + `dist/index.d.mts` (and the `hooks`
 * pair) from `src/`, which the published `exports` map points at.
 * `@tynd/core` is a peer dep — declared external so consumers' own copy
 * is used at runtime instead of being inlined into our bundle.
 */
export default defineBuildConfig({
  entries: ["src/index", "src/hooks"],
  outDir: "dist",
  // `node16` emits only `.d.mts` (+ `.d.cts` for CJS consumers); the
  // default `compatible` ALSO ships a duplicate `.d.ts` payload that
  // nothing in `package.json#exports` references — wasted bytes in the
  // tarball. Our exports map points at `.d.mts` exclusively, so go
  // node16-only.
  declaration: "node16",
  clean: true,
  rollup: {
    emitCJS: false,
    inlineDependencies: false,
  },
  externals: [
    "@tynd/core",
    "@tynd/core/client",
    "@tynd/core/client/bridge",
    "@tynd/core/plugin",
    "hookable",
  ],
  failOnWarn: false,
});
