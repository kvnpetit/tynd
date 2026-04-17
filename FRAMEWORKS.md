# Supported frontend frameworks

Tynd is framework-agnostic at runtime — the frontend is a plain folder of static assets served over `bv://` in production (or a dev server in development). This document covers **which frameworks the CLI knows how to scaffold, detect, and drive**, and **which are blocked**, with per-framework notes.

> Last updated: April 16, 2026

---

## Legend

| Symbol | Meaning |
|---|---|
| ✅ | Works |
| ⚠ | Works but with a caveat — see notes |
| ♻ | Full page reload only (no component-level Fast Refresh) |
| ❌ | Does not work / blocked |

---

## Summary matrix

Scaffold and production build have been verified for each row. The HMR column reflects the **default** experience after `tynd create`.

| Framework  | `tynd create` | `tynd build` | Binary launches | Fast Refresh (HMR) | Scaffold source |
|---|---|---|---|---|---|
| **React**   | ✅ | ✅ | ✅ | ⚠ OK; breaks if [React Compiler](#react-compiler) is enabled | Vite `react-ts`     |
| **Vue**     | ✅ | ✅ | ✅ | ✅ (`@vitejs/plugin-vue`)                       | Vite `vue-ts`       |
| **Svelte**  | ✅ | ✅ | ✅ | ✅ (`@sveltejs/vite-plugin-svelte`)             | Vite `svelte-ts`    |
| **Solid**   | ✅ | ✅ | ✅ | ✅ (`vite-plugin-solid`)                        | Vite `solid-ts`     |
| **Preact**  | ✅ | ✅ | ✅ | ✅ (`@preact/preset-vite` / Prefresh)           | Vite `preact-ts`    |
| **Lit**     | ✅ | ✅ | ✅ | ♻ Full reload — [by design](#lit)               | Vite `lit-ts`       |
| **Angular** | ✅ | ✅ | ✅ | ♻ Full reload by default — [see note](#angular) | Angular CLI         |

```bash
tynd create my-app --framework <react|vue|svelte|solid|preact|lit|angular> --runtime <full|lite>
```

**End-to-end verification.** For each row above, a fresh project was scaffolded in `playground/test-<framework>/`, deps resolved via workspace links, and `tynd build` produced a runnable `lite` binary (~2.5 MB) with a valid `TYNDPKG\0` trailer. Each binary was launched and stayed alive for 2 s (no startup crash). React is additionally exercised by the long-running `playground/full` and `playground/lite` apps.

The Fast Refresh column reflects upstream plugin behavior — the dev server is not instrumented by Tynd, it's the plain Vite / Angular CLI dev server that the framework already uses outside Tynd.

---

## Build tools detected by `tynd init`

`tynd init` adds Tynd to an **existing** project by inspecting `package.json`. It identifies the build tool from the dependency graph and fills `frontendDir` / dev command accordingly.

| Build tool | Trigger dep | `devUrl` (default) | `outDir` (default) | Notes |
|---|---|---|---|---|
| **Vite**        | `vite` or any `@vitejs/*` | `http://localhost:5173` | `dist`                      | Reads `outDir` from `vite.config.(ts\|js\|mts\|mjs)` if overridden |
| **Create React App** | `react-scripts`      | `http://localhost:3000` | `build`                     | Pure SPA only |
| **Angular CLI** | `@angular/cli` or `@angular-devkit/build-angular` | `http://localhost:4200` | `dist/<project>/browser` (for `@angular/build:application`) | Reads `outputPath` + `builder` from `angular.json`; falls back to `dist/<project>` for non-application builders |
| **Parcel**      | `parcel` or `parcel-bundler` | `http://localhost:1234` | `dist`                      | — |
| **Rsbuild**     | `@rsbuild/core`       | `http://localhost:3000` | `dist`                      | Reads `distPath.root` from `rsbuild.config.(ts\|js)` |
| **Webpack**     | `webpack` or `webpack-cli` | `http://localhost:8080` | `dist`                      | Assumes `webpack-dev-server` on :8080 |

Source: [`packages/cli/src/lib/detect.ts`](packages/cli/src/lib/detect.ts).

---

## Blocked — SSR / server-owning frameworks

Tynd owns the HTTP layer (`bv://` custom protocol in prod, dev server URL in dev) and packages the app as a **pure SPA**. Server-side frameworks that need to run Node/Deno/Bun on the end-user's machine are incompatible. `tynd init` and `tynd dev` exit with an error when any of these are found in `dependencies` / `devDependencies`:

| Framework | Trigger dep(s) | SPA alternative |
|---|---|---|
| **Next.js**           | `next`                                                        | Use Vite + React (or Tauri/Electron) |
| **Nuxt**              | `nuxt`, `@nuxtjs/bridge`                                      | Use Vite + Vue |
| **SvelteKit**         | `@sveltejs/kit`                                               | Plain Svelte (`tynd create … -f svelte`) |
| **Remix**             | `@remix-run/react`, `@remix-run/node`, `@remix-run/cloudflare`, `@remix-run/deno` | Vite + React Router SPA |
| **Gatsby**            | `gatsby`                                                      | Any other SSG -> static `dist/` |
| **Blitz.js**          | `blitz`                                                       | — |
| **RedwoodJS**         | `@redwoodjs/core`                                             | — |
| **SolidStart**        | `@solidjs/start`                                              | Plain Solid (`tynd create … -f solid`) |
| **Angular Universal** | `@angular/platform-server`, `@nguniversal/express-engine`     | Plain Angular (`tynd create … -f angular`) |
| **Analog**            | `@analogjs/core`, `@analogjs/platform`                        | Plain Angular |
| **Qwik City**         | `@builder.io/qwik-city`                                       | Plain Qwik (not currently scaffolded; use `tynd init`) |
| **Astro**             | `astro`                                                       | Any other SSG |
| **TanStack Start**    | `@tanstack/start`, `@tanstack/react-start`                    | Vite + TanStack Router SPA |
| **Vike**              | `vike`, `vite-plugin-ssr`                                     | Vite + SPA |

Source of truth: `SERVER_FRAMEWORKS` in [`packages/cli/src/lib/detect.ts`](packages/cli/src/lib/detect.ts).

---

## Per-framework notes

### React

- Scaffolded via `bun create vite@latest <name> --template react-ts` -> React 19 + TypeScript.
- HMR via `@vitejs/plugin-react@6` (oxc-based Fast Refresh).
- ESLint flat config + `eslint-plugin-react-hooks` + `eslint-plugin-react-refresh` included.

#### React Compiler

The Vite template ships `babel-plugin-react-compiler` and `@rolldown/plugin-babel` as devDependencies, and the generated `vite.config.ts` wires them after `react()`:

```ts
plugins: [react(), babel({ presets: [reactCompilerPreset()] })]
```

**This breaks HMR in dev.** `@vitejs/plugin-react@6` uses oxc to inject Fast Refresh markers (`$RefreshReg$`, `$RefreshSig$`); `@rolldown/plugin-babel` then re-parses the output and strips them. Vite still emits `[vite] (client) hmr update`, but no refresh boundary is registered, so the WebView shows nothing until a manual reload.

Playground templates in this repo (`playground/full`, `playground/lite`) **do not** enable the Compiler for this reason.

If you want the Compiler, enable it **only in build mode** so dev HMR stays intact:

```ts
// vite.config.ts
import { defineConfig } from 'vite'
import react, { reactCompilerPreset } from '@vitejs/plugin-react'
import babel from '@rolldown/plugin-babel'

export default defineConfig(({ command }) => ({
  plugins: [
    react(),
    ...(command === 'build'
      ? [babel({ presets: [reactCompilerPreset()] })]
      : []),
  ],
}))
```

Tradeoff: dev components are not memoized (matches every pre-Compiler React app). Production output is.

> Alternative narrow path: `reactCompilerPreset({ compilationMode: 'annotation' })` + `"use memo"` directive on opt-in components. Still runs babel on those files in dev, so HMR can still misbehave on annotated files — use at your own risk.

---

### Vue

- Scaffolded via `bun create vite@latest <name> --template vue-ts` -> Vue 3 + TypeScript + `vue-tsc`.
- HMR via `@vitejs/plugin-vue`. No known issues.
- `build:ui` script uses `vue-tsc -b && vite build`.

---

### Svelte

- Scaffolded via `bun create vite@latest <name> --template svelte-ts` -> Svelte 5 + TypeScript + `svelte-check`.
- HMR via `@sveltejs/vite-plugin-svelte`. No known issues.
- **Do not install `@sveltejs/kit`** — it is SSR and will be rejected by `tynd init` / `tynd dev` / `tynd build`.

---

### Solid

- Scaffolded via `bun create vite@latest <name> --template solid-ts` -> Solid 1.x + TypeScript.
- HMR via `vite-plugin-solid`. No known issues.
- **Do not install `@solidjs/start`** — SSR framework, blocked.

---

### Preact

- Scaffolded via `bun create vite@latest <name> --template preact-ts` -> Preact + TypeScript.
- HMR via `@preact/preset-vite`. Uses Preact's own refresh implementation (separate from React's Fast Refresh) — the React Compiler caveat above does **not** apply here.

---

### Lit

- Scaffolded via `bun create vite@latest <name> --template lit-ts` -> Lit 3 + TypeScript.
- **No Fast Refresh.** Vite performs a full page reload on any change. This is intentional: Web Components persist state in custom element registrations and shadow DOM, and cannot be hot-swapped safely. Every framework that ships Web Components (Lit, Stencil, plain `customElements.define`) behaves this way.
- Not a Tynd limitation — the same applies in a plain Vite + Lit project outside Tynd.

---

### Angular

- Scaffolded via `bunx @angular/cli@latest new <name> --defaults --skip-git --skip-install --ssr=false`.
- Dev server: `bunx ng serve` on `http://localhost:4200`.
- **HMR behavior.** `ng serve` **full-reloads by default** — not Fast Refresh. Angular does support HMR via `ng serve --hmr`, but Tynd does not pass that flag out of the box. If you want component-level hot-reload, add it yourself in `tynd.config.ts`:

  ```ts
  export default {
    // ...
    devCommand: "bunx ng serve --hmr",
  } satisfies TyndConfig
  ```
  The HMR reliability in Angular is also framework-version-dependent; test before relying on it.
- **`frontendDir` resolution:** Tynd reads `angular.json`, finds the first `projectType: "application"`, and combines `outputPath` (default: `dist/<project>`) with the builder:
  - `@angular/build:application` (Angular 17+) -> `<outputPath>/browser`
  - anything else -> `<outputPath>`
- **Do not install `@angular/platform-server`, `@nguniversal/express-engine`, or `@analogjs/*`** — SSR variants, blocked.

---

## Adding a new framework

Two extension points:

1. **`FRAMEWORKS` array** in [`packages/cli/src/commands/create.ts`](packages/cli/src/commands/create.ts) — controls what `tynd create` can scaffold. For Vite-backed frameworks, add an entry with `viteTemplate: "<name>-ts"`. For frameworks with their own CLI (like Angular), set `viteTemplate: null` and add a branch in `create()` that shells out to that CLI.
2. **`detectFrontend` + `TOOL_COMMANDS` + `DEFAULT_OUT_DIR`** in [`packages/cli/src/lib/detect.ts`](packages/cli/src/lib/detect.ts) — controls what `tynd init` / `tynd dev` / `tynd build` recognize in an existing project. Add a dep -> tool mapping plus the dev/build commands and default output directory. If the framework has a config file encoding a custom output directory, also extend `resolveOutDir`.

If the new framework is SSR (owns a server process at end-user runtime), add its entry package to `SERVER_FRAMEWORKS` instead — it will fail-fast with a clear message directing to the SPA alternative.
