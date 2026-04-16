/**
 * Bun build for the vorn-lite backend.
 *
 * Bun does not support esbuild's `--global-name` flag so we cannot build IIFE
 * directly. Instead we build in ESM format and rewrite the trailing
 * `export { a, b, c };` to `globalThis.__vorn_mod__ = { a, b, c };` so the
 * embedded runtime can read the module exports from globalThis.
 */

import { log } from "./logger.ts"

/**
 * Build a simple frontend TypeScript/JavaScript entry point.
 * Used for projects without a framework (no Vite, no CRA, etc.).
 * Output goes to `outDir/<basename>.js`.
 */
export async function buildFrontendEntry(entryPath: string, outDir: string): Promise<void> {
  log.debug(`Bun.build frontend: ${entryPath} → ${outDir}`)
  const result = await Bun.build({
    entrypoints: [entryPath],
    outdir: outDir,
    target: "browser",
  })

  if (!result.success) {
    const msgs = result.logs.map((l) => l.message).join("\n")
    throw new Error(`Frontend build failed:\n${msgs}`)
  }
}

/**
 * Build the full-mode backend bundle for distribution.
 * Target: "bun" — produces a self-contained JS file runnable by `bun run`.
 */
export async function buildFullBundle(entryPath: string, outPath: string): Promise<void> {
  log.debug(`Bun.build full: ${entryPath} → ${outPath} (minify=true)`)
  const result = await Bun.build({
    entrypoints: [entryPath],
    target: "bun",
    format: "esm",
    minify: true,
    define: { "globalThis.__VORN_RUNTIME__": '"full"' },
  })

  if (!result.success) {
    const msgs = result.logs.map((l) => l.message).join("\n")
    throw new Error(`Full bundle failed:\n${msgs}`)
  }

  const output = result.outputs[0]
  if (!output) throw new Error("Bun.build produced no output files")
  const code = await output.text()
  await Bun.write(outPath, code)
}

/** Build the lite backend bundle and write it to `outPath`. */
export async function buildLiteBundle(
  entryPath: string,
  outPath: string,
  minify: boolean,
  dev = false,
): Promise<void> {
  log.debug(`Bun.build lite: ${entryPath} → ${outPath} (minify=${minify}, dev=${dev})`)
  const result = await Bun.build({
    entrypoints: [entryPath],
    target: "browser",
    format: "esm",
    minify,
    define: {
      "globalThis.__VORN_RUNTIME__": '"lite"',
      ...(dev ? { "globalThis.__VORN_DEV__": "true" } : {}),
    },
  })

  if (!result.success) {
    const msgs = result.logs.map((l) => l.message).join("\n")
    throw new Error(`Bundle failed:\n${msgs}`)
  }

  const output = result.outputs[0]
  if (!output) throw new Error("Bun.build produced no output files")
  let code = await output.text()

  // QuickJS evaluates the bundle as a script (not an ES module), so `import.meta`
  // is a SyntaxError. Replace every `import.meta` with a plain object before eval.
  // In lite mode, `import.meta.path` is only passed as `app.start({ entry })` which
  // _startLite() ignores, so replacing with {} is safe.
  code = code.replace(/\bimport\.meta\b/g, "({})")

  await Bun.write(outPath, esmToGlobal(code, "__vorn_mod__"))
}

export function esmToGlobal(code: string, globalName: string): string {
  const exports: string[] = []

  // Replace ALL named export statements and collect their bindings
  const stripped = code.replace(/\bexport\s*\{([^}]*)\}\s*;?/gm, (_, block: string) => {
    for (const entry of block.split(",")) {
      const trimmed = entry.trim()
      if (!trimmed) continue
      const parts = trimmed.split(/\s+as\s+/)
      const local = parts[0]!.trim()
      const exported = (parts[1] ?? local).trim()
      exports.push(`  ${exported}: ${local},`)
    }
    return ""
  })

  if (exports.length === 0) {
    throw new Error(
      "[bundle] No ESM export block found — bundle may not expose the backend module.",
    )
  }

  return `${stripped.trimEnd()}\n${globalName} = {\n${exports.join("\n")}\n};\n`
}
