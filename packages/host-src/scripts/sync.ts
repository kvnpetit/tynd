#!/usr/bin/env bun
/**
 * Copies `packages/host-rs/` into `packages/host-src/native/`, flattening
 * every `workspace = true` reference to an explicit version/value read from
 * the root `Cargo.toml`. The output is a standalone crate that builds
 * inside the `.tynd/build/` workspace the CLI generates for custom
 * runtimes.
 *
 * Intentionally hand-rolled: we only need to rewrite a handful of
 * well-known patterns, and the dependency graph is too tight to justify
 * pulling in a full TOML parser.
 */

import { existsSync, mkdirSync, readdirSync, rmSync, statSync } from "node:fs"
import { dirname, join, relative } from "node:path"

const ROOT = join(import.meta.dir, "..", "..", "..")
const HOST_RS = join(ROOT, "packages", "host-rs")
const HOST_SRC_NATIVE = join(import.meta.dir, "..", "native")
const WORKSPACE_TOML = join(ROOT, "Cargo.toml")
const HOST_RS_TOML = join(HOST_RS, "Cargo.toml")

type WorkspaceValues = {
  pkg: Record<string, string>
  deps: Record<string, string>
}

async function readWorkspaceValues(): Promise<WorkspaceValues> {
  const text = await Bun.file(WORKSPACE_TOML).text()

  const pkg: Record<string, string> = {}
  const deps: Record<string, string> = {}

  const sections = splitSections(text)

  for (const [header, body] of sections) {
    if (header === "workspace.package") {
      for (const [k, v] of parseKv(body)) pkg[k] = v
    } else if (header === "workspace.dependencies") {
      for (const [k, v] of parseKv(body)) deps[k] = v
    }
  }

  return { pkg, deps }
}

type Section = [header: string, body: string]

function splitSections(toml: string): Section[] {
  const out: Section[] = []
  let header = ""
  let body: string[] = []
  for (const line of toml.split(/\r?\n/)) {
    const m = line.match(/^\s*\[([^\]]+)\]\s*$/)
    if (m) {
      if (header) out.push([header, body.join("\n")])
      header = m[1]!.trim()
      body = []
    } else {
      body.push(line)
    }
  }
  if (header) out.push([header, body.join("\n")])
  return out
}

/** Parse `key = value` pairs, skipping blank/comment lines. */
function parseKv(body: string): Array<[string, string]> {
  const out: Array<[string, string]> = []
  for (const raw of body.split(/\r?\n/)) {
    const line = raw.replace(/\s+#.*$/, "").trim()
    if (!line) continue
    const eq = line.indexOf("=")
    if (eq === -1) continue
    const key = line.slice(0, eq).trim()
    const value = line.slice(eq + 1).trim()
    out.push([key, value])
  }
  return out
}

function flattenCargoToml(src: string, ws: WorkspaceValues): string {
  const lines = src.split(/\r?\n/)
  const out: string[] = []
  let inPackage = false
  let skippingSection = false
  for (const raw of lines) {
    const sectionMatch = raw.match(/^\s*\[([^\]]+)\]\s*$/)
    if (sectionMatch) {
      const name = sectionMatch[1]!.trim()
      inPackage = name === "package"
      // [lints] inherits from workspace — doesn't apply to the
      // standalone crate shipped to users; strip it.
      skippingSection = name === "lints"
      if (skippingSection) continue
      out.push(raw)
      continue
    }
    if (skippingSection) continue

    const stripped = raw.replace(/\s+#.*$/, "")
    const trimmed = stripped.trim()

    // `readme.workspace = true` / `readme = "../../README.md"` — the path
    // escapes the crate dir once vendored into `.tynd/build/host/`. Drop it.
    if (inPackage && /^readme\b/.test(trimmed)) continue

    // `foo.workspace = true` — top-level inheritance.
    const shortForm = trimmed.match(/^([A-Za-z0-9_-]+)\.workspace\s*=\s*true\s*$/)
    if (shortForm) {
      const key = shortForm[1]!
      const replacement = inPackage ? workspacePkgValue(ws, key) : workspaceDepShortValue(ws, key)
      if (replacement !== null) {
        out.push(raw.replace(trimmed, replacement))
        continue
      }
    }

    // `foo = { workspace = true, ... }` — long form with extra flags.
    const longForm = trimmed.match(
      /^([A-Za-z0-9_-]+)\s*=\s*\{\s*workspace\s*=\s*true\s*(,.*)?\s*\}\s*$/,
    )
    if (longForm) {
      const key = longForm[1]!
      const extras = (longForm[2] ?? "")
        .replace(/^,\s*/, "")
        .replace(/\s*\}\s*$/, "")
        .trim()
      const version = workspaceDepVersion(ws, key)
      if (version !== null) {
        const extrasSuffix = extras ? `, ${extras}` : ""
        out.push(
          raw.replace(
            trimmed,
            `${key} = { version = "${version}"${extrasSuffix}${mergeExtrasFromWorkspace(ws, key)} }`,
          ),
        )
        continue
      }
    }

    out.push(raw)
  }
  return out.join("\n")
}

function workspacePkgValue(ws: WorkspaceValues, key: string): string | null {
  const v = ws.pkg[key]
  if (!v) return null
  return `${key} = ${v}`
}

function workspaceDepShortValue(ws: WorkspaceValues, key: string): string | null {
  const raw = ws.deps[key]
  if (!raw) return null
  if (raw.startsWith('"')) {
    return `${key} = ${raw}`
  }
  // Inline table like `{ version = "1.0", features = [...] }` — just copy verbatim.
  return `${key} = ${raw}`
}

function workspaceDepVersion(ws: WorkspaceValues, key: string): string | null {
  const raw = ws.deps[key]
  if (!raw) return null
  if (raw.startsWith('"')) {
    return raw.slice(1, -1)
  }
  const m = raw.match(/version\s*=\s*"([^"]+)"/)
  return m ? m[1]! : null
}

/**
 * When the workspace dep is an inline table with extra fields (features,
 * default-features, etc.), forward those into the per-crate long form too.
 */
function mergeExtrasFromWorkspace(ws: WorkspaceValues, key: string): string {
  const raw = ws.deps[key]
  if (!raw || raw.startsWith('"')) return ""
  const parts: string[] = []
  const features = raw.match(/features\s*=\s*(\[[^\]]*\])/)
  if (features) parts.push(`features = ${features[1]!}`)
  const defaultFeatures = raw.match(/default-features\s*=\s*(true|false)/)
  if (defaultFeatures) parts.push(`default-features = ${defaultFeatures[1]!}`)
  return parts.length ? `, ${parts.join(", ")}` : ""
}

function copyDir(from: string, to: string) {
  mkdirSync(to, { recursive: true })
  for (const entry of readdirSync(from)) {
    const src = join(from, entry)
    const dst = join(to, entry)
    const st = statSync(src)
    if (st.isDirectory()) {
      copyDir(src, dst)
    } else {
      mkdirSync(dirname(dst), { recursive: true })
      // Preserve byte-exact content via Bun.write.
      const data = Bun.file(src)
      Bun.write(dst, data)
    }
  }
}

if (!existsSync(HOST_RS_TOML)) {
  throw new Error(`host-rs not found at ${HOST_RS_TOML}`)
}

const ws = await readWorkspaceValues()
const rawHostToml = await Bun.file(HOST_RS_TOML).text()
const flattened = flattenCargoToml(rawHostToml, ws)

if (existsSync(HOST_SRC_NATIVE)) {
  rmSync(HOST_SRC_NATIVE, { recursive: true, force: true })
}
mkdirSync(HOST_SRC_NATIVE, { recursive: true })

// Source tree — copy verbatim.
copyDir(join(HOST_RS, "src"), join(HOST_SRC_NATIVE, "src"))

// Flattened Cargo.toml — preserves crate name + features + optional deps.
await Bun.write(join(HOST_SRC_NATIVE, "Cargo.toml"), flattened)

const ok = relative(ROOT, HOST_SRC_NATIVE)
console.log(`✓ synced host-rs -> ${ok}`)
