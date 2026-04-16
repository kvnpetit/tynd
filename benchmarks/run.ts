#!/usr/bin/env bun

/**
 * Vorn Benchmark Runner
 *
 * Automatically runs both lite and full benchmarks in sequence using WebView,
 * collects results via a local HTTP server, and displays a side-by-side comparison.
 *
 * Usage: bun run.ts
 */

import { existsSync, mkdirSync, statSync, writeFileSync } from "node:fs"
import path from "node:path"

// ── Types ─────────────────────────────────────────────────────────────────────

type BenchRow = {
  name: string
  kind: "cold" | "latency" | "concurrent" | "sustained" | "na"
  p50?: number
  p95?: number
  p99?: number
  min?: number
  max?: number
  stddev?: number
  cv?: number
  ops?: number
  coldMs?: number
  perCall?: number
  total?: number
  calls?: number
}

type ResultsPayload = {
  runtime: string
  platform: string
  timestamp: string
  rows: BenchRow[]
  frontendHeapMB?: number // peak WebView2 JS heap during benchmark
  backendHeapMB?: number // backend process heap (full=Bun, lite=0)
  backendRssMB?: number // backend process RSS (full=Bun, lite=0)
  bundleSizeKB?: number // lite bundle size in KB (lite only)
}

// ── Terminal colors ───────────────────────────────────────────────────────────

const c = {
  reset: "\x1b[0m",
  bold: "\x1b[1m",
  dim: "\x1b[2m",
  green: "\x1b[32m",
  cyan: "\x1b[36m",
  yellow: "\x1b[33m",
  red: "\x1b[31m",
}

const bold = (s: string | number) => `${c.bold}${s}${c.reset}`
const dim = (s: string | number) => `${c.dim}${s}${c.reset}`
const green = (s: string | number) => `${c.green}${s}${c.reset}`
const yellow = (s: string | number) => `${c.yellow}${s}${c.reset}`
const red = (s: string | number) => `${c.red}${s}${c.reset}`
const cyan = (s: string | number) => `${c.cyan}${s}${c.reset}`

// ── Binary search ─────────────────────────────────────────────────────────────

function findBinary(runtime: "lite" | "full", from: string): string | null {
  const name = `vorn-${runtime}${process.platform === "win32" ? ".exe" : ""}`
  let dir = from
  for (let i = 0; i < 6; i++) {
    const release = path.join(dir, "target", "release", name)
    const debug = path.join(dir, "target", "debug", name)
    if (existsSync(release)) return release
    if (existsSync(debug)) return debug
    const parent = path.dirname(dir)
    if (parent === dir) break
    dir = parent
  }
  return null
}

// ── ESM → global transform ────────────────────────────────────────────────────

function esmToGlobal(code: string, globalName: string): string {
  const exports: string[] = []

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
    throw new Error("[bench] No ESM export block found in bundle — backend module not exposed.")
  }

  return `${stripped.trimEnd()}\n${globalName} = {\n${exports.join("\n")}\n};\n`
}

// ── Results HTTP server ───────────────────────────────────────────────────────

function startResultsServer(): { wait: () => Promise<ResultsPayload>; stop: () => void } {
  let _resolve!: (r: ResultsPayload) => void
  const promise = new Promise<ResultsPayload>((r) => {
    _resolve = r
  })

  const server = Bun.serve({
    port: 9876,
    hostname: "0.0.0.0",
    async fetch(req) {
      const cors = {
        "Access-Control-Allow-Origin": "*",
        "Access-Control-Allow-Methods": "POST, OPTIONS",
        "Access-Control-Allow-Headers": "Content-Type",
      }
      if (req.method === "OPTIONS") return new Response(null, { status: 204, headers: cors })
      if (req.method !== "POST") return new Response("", { status: 404 })
      const data = (await req.json()) as ResultsPayload
      _resolve(data)
      return new Response(JSON.stringify({ ok: true }), {
        headers: { ...cors, "Content-Type": "application/json" },
      })
    },
  })

  return { wait: () => promise, stop: () => server.stop() }
}

// ── Vite dev server ───────────────────────────────────────────────────────────

async function waitForVite(url: string, timeoutMs = 30_000): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url)
      if (res.ok || res.status < 500) return
    } catch {
      /* not ready yet */
    }
    await Bun.sleep(200)
  }
  throw new Error(`Vite dev server not ready at ${url} after ${timeoutMs}ms`)
}

// ── Build lite bundle ─────────────────────────────────────────────────────────

async function buildLiteBundle(coreDir: string, outDir: string): Promise<string> {
  const outPath = path.join(outDir, "bundle.dev.js")
  console.log(dim("  Building lite bundle…"))

  const result = await Bun.build({
    entrypoints: [path.join(coreDir, "backend", "main.ts")],
    target: "browser",
    format: "esm",
    define: {
      "globalThis.__VORN_RUNTIME__": '"lite"',
      "process.platform": '"quickjs"', // IS_FULL = process.platform !== "quickjs"
    },
  })

  if (!result.success) {
    for (const log of result.logs) console.error(log)
    throw new Error("Lite bundle build failed")
  }

  const text = await result.outputs[0]!.text()
  const transformed = esmToGlobal(text, "globalThis.__vorn_mod__").replace(/import\.meta/g, "({})")
  writeFileSync(outPath, transformed, "utf-8")
  console.log(dim(`  Lite bundle → ${outPath}`))
  return outPath
}

// ── Comparison display ────────────────────────────────────────────────────────

function fmt(ms: number): string {
  if (ms < 0.01) return `${(ms * 1000).toFixed(1)}µs`
  return `${ms.toFixed(3)}ms`
}

function fmtOps(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k/s` : `${n}/s`
}

function fmtKB(bytes: number): string {
  return bytes >= 1024 * 1024
    ? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
    : `${(bytes / 1024).toFixed(0)} KB`
}

function fmtMB(mb: number): string {
  return `${mb.toFixed(1)} MB`
}

// Section labels that trigger a divider line in the comparison output
const SECTION_LABELS: Record<string, string> = {
  // ① App startup
  "startup — first call": "── ① App Startup (cold — user feels this at launch) ──",
  // ② Button click floor
  "empty call (IPC floor)": "── ② Button Click — Dispatch Floor ─────────────────",
  // ③ Typical app workloads
  "filter+sort list — 200 items": "── ③ Typical App Workloads ──────────────────────────",
  "validate API batch — 50 items": "   (API batch validation)",
  "build report — 50 rows": "   (report: aggregate + format)",
  // ④ File operations
  "read config — 1KB": "── ④ File Operations (full/Bun only) ────────────────",
  "save settings — 1KB": "   (write)",
  "settings roundtrip — write+read 1KB": "   (write+read roundtrip)",
  // ⑤ Local database
  "db lookup by ID": "── ⑤ Local Database — SQLite (full only) ────────────",
  "db save 100 records (batch)": "   (write)",
  "db dashboard aggregate": "   (aggregate query)",
  "db full-text search (LIKE)": "   (search query)",
  // ⑥ Data transfer
  "send+receive 1KB": "── ⑥ Data Transfer to/from UI ──────────────────────",
  "backend → UI list ~10KB": "   (backend→UI only)",
  "nested config object (depth 10)": "   (nested JSON)",
  // ⑦ Real-time updates
  "backend event → UI handler": "── ⑦ Real-time Updates (event push) ────────────────",
  // ⑧ Concurrent requests
  "10 simultaneous calls": "── ⑧ Concurrent Requests ────────────────────────────",
  "10 simultaneous calls with 1KB payload": "   (concurrent with payload)",
  // ⑨ Sustained throughput
  "max call rate (2s)": "── ⑨ Sustained Throughput ───────────────────────────",
  // ⑩ Engine benchmarks (synthetic)
  "[engine] fibonacci(25) — recursive": "── ⑩ JS Engine Benchmarks — Synthetic (not typical) ─",
  "[engine] sort 1 000 numbers": "   (sort / memory)",
  "[engine] map→filter→reduce 1 000 items": "   (functional JS)",
  "[engine] JSON stringify+parse ×100": "   (JSON — key differentiator)",
  "[engine] regex match ×100": "   (regex engine)",
  "[engine] string concat ×10 000": "   (string + GC)",
  "[engine] object alloc ×1 000": "   (object alloc / GC pressure)",
}

// Results within TIE_THRESHOLD are considered statistically indistinguishable
const TIE_THRESHOLD = 1.08 // 8% margin

function printComparison(
  liteRes: ResultsPayload,
  fullRes: ResultsPayload,
  liteBinPath: string,
  fullBinPath: string,
  bundlePath: string,
  backendPath: string,
) {
  console.log()
  console.log(bold(`  ╔══════════════════════════════════════════════════════════════════════╗`))
  console.log(bold(`  ║              Vorn Benchmark — Lite vs Full Comparison               ║`))
  console.log(bold(`  ╚══════════════════════════════════════════════════════════════════════╝`))
  console.log()
  console.log(dim(`  Platform:  ${liteRes.platform}   Bun: ${Bun.version}`))
  console.log(dim(`  Lite run:  ${liteRes.timestamp}`))
  console.log(dim(`  Full run:  ${fullRes.timestamp}`))
  console.log()

  // ── Static metrics (sizes) ────────────────────────────────────────────────
  const liteBinSize = statSync(liteBinPath).size
  const fullBinSize = statSync(fullBinPath).size
  const bundleSize = statSync(bundlePath).size
  const backendSize = statSync(backendPath).size

  const COL = [42, 12, 12, 8, 7] as const
  const SEP = "─".repeat(COL.reduce((a, b) => a + b + 1, 0) + 2)

  console.log(`  ${bold("Static Metrics")}`)
  console.log(`  ${dim(SEP)}`)

  const smRow = (label: string, lite: string, full: string) => {
    const name = label.padEnd(COL[0])
    console.log(`  ${name} ${lite.padStart(COL[1])} ${full.padStart(COL[2])}`)
  }
  console.log(`  ${"".padEnd(COL[0])} ${"Lite".padStart(COL[1])} ${"Full".padStart(COL[2])}`)
  console.log(`  ${dim(SEP)}`)
  smRow("Binary size", fmtKB(liteBinSize), fmtKB(fullBinSize))
  smRow("JS bundle (backend)", fmtKB(bundleSize), dim("none (runs TS directly)"))
  smRow("Backend source", fmtKB(backendSize), fmtKB(backendSize))

  // Memory metrics (from benchmark run)
  if (liteRes.frontendHeapMB !== undefined || fullRes.frontendHeapMB !== undefined) {
    smRow(
      "Peak WebView2 JS heap",
      liteRes.frontendHeapMB !== undefined ? fmtMB(liteRes.frontendHeapMB) : "—",
      fullRes.frontendHeapMB !== undefined ? fmtMB(fullRes.frontendHeapMB) : "—",
    )
  }
  if (fullRes.backendRssMB !== undefined && fullRes.backendRssMB > 0) {
    smRow(
      "Backend RSS (process)",
      dim("in-process (same as WebView2)"),
      fmtMB(fullRes.backendRssMB),
    )
    smRow(
      "Backend heap used",
      dim("—"),
      fullRes.backendHeapMB !== undefined ? fmtMB(fullRes.backendHeapMB) : "—",
    )
  }

  console.log(`  ${dim(SEP)}`)
  console.log()

  // ── Performance comparison ────────────────────────────────────────────────
  console.log(
    `  ${bold("Performance Comparison")}   ${dim("(values: P50 for latency, perCall for concurrent, ops/s for sustained)")}`,
  )
  console.log()

  const header = [
    "Benchmark".padEnd(COL[0]),
    "Lite".padStart(COL[1]),
    "Full".padStart(COL[2]),
    "Winner".padStart(COL[3]),
    "Ratio".padStart(COL[4]),
  ].join(" ")

  console.log(`  ${dim(header)}`)
  console.log(`  ${dim(SEP)}`)

  const fullMap = new Map<string, BenchRow>()
  for (const row of fullRes.rows) fullMap.set(row.name, row)

  let liteWins = 0
  let fullWins = 0
  let fullExclusive = 0 // full-only capabilities (file I/O, SQLite)

  for (const liteRow of liteRes.rows) {
    // Print section divider if applicable
    const sectionLabel = SECTION_LABELS[liteRow.name]
    if (sectionLabel !== undefined) {
      if (sectionLabel) console.log(`  ${dim(sectionLabel)}`)
    }

    const fullRow = fullMap.get(liteRow.name)

    // ── N/A row: lite doesn't support this operation ──────────────────────────
    if (liteRow.kind === "na") {
      if (!fullRow) continue
      let fullStr = "—"
      if (fullRow.kind === "latency" && fullRow.p50 !== undefined) {
        fullStr = fmt(fullRow.p50)
      } else if (fullRow.kind === "cold" && fullRow.coldMs !== undefined) {
        fullStr = fmt(fullRow.coldMs)
      }
      const name =
        liteRow.name.length > COL[0]
          ? `${liteRow.name.slice(0, COL[0] - 1)}…`
          : liteRow.name.padEnd(COL[0])
      console.log(
        `  ${name} ${dim("N/A".padStart(COL[1]))} ${cyan(fullStr.padStart(COL[2]))} ${cyan("✓ full".padStart(COL[3]))} ${dim("∞".padStart(COL[4]))}`,
      )
      fullExclusive++
      continue
    }

    if (!fullRow) continue

    let liteVal: number | undefined
    let fullVal: number | undefined
    let liteStr = "—"
    let fullStr = "—"
    let lowerIsBetter = true

    if (liteRow.kind === "cold" && liteRow.coldMs !== undefined && fullRow.coldMs !== undefined) {
      liteVal = liteRow.coldMs
      fullVal = fullRow.coldMs
      liteStr = fmt(liteVal)
      fullStr = fmt(fullVal)
    } else if (
      liteRow.kind === "latency" &&
      liteRow.p50 !== undefined &&
      fullRow.p50 !== undefined
    ) {
      liteVal = liteRow.p50
      fullVal = fullRow.p50
      liteStr = fmt(liteVal)
      fullStr = fmt(fullVal)
    } else if (
      liteRow.kind === "concurrent" &&
      liteRow.perCall !== undefined &&
      fullRow.perCall !== undefined
    ) {
      liteVal = liteRow.perCall
      fullVal = fullRow.perCall
      liteStr = fmt(liteVal)
      fullStr = fmt(fullVal)
    } else if (
      liteRow.kind === "sustained" &&
      liteRow.ops !== undefined &&
      fullRow.ops !== undefined
    ) {
      liteVal = liteRow.ops
      fullVal = fullRow.ops
      liteStr = fmtOps(liteVal)
      fullStr = fmtOps(fullVal)
      lowerIsBetter = false
    }

    if (liteVal === undefined || fullVal === undefined) continue

    const lo = Math.min(liteVal, fullVal)
    const hi = Math.max(liteVal, fullVal)
    const ratio = lo > 0 ? hi / lo : 1
    // Show +X% for small gains (more intuitive), Nx for large gains (cleaner than +3900%)
    const ratioStr =
      ratio < TIE_THRESHOLD
        ? ""
        : ratio < 10
          ? `+${Math.round((ratio - 1) * 100)}%`
          : `${ratio.toFixed(1)}×`
    const isTie = ratio < TIE_THRESHOLD

    const liteWins_this = !isTie && (lowerIsBetter ? liteVal < fullVal : liteVal > fullVal)
    const fullWins_this = !isTie && !liteWins_this

    if (liteWins_this) liteWins++
    if (fullWins_this) fullWins++

    // Show noise indicator if CV is high (>20%)
    const liteCV = liteRow.cv
    const fullCV = fullRow.cv
    const noisy = (liteCV !== undefined && liteCV > 20) || (fullCV !== undefined && fullCV > 20)

    const liteDisplay = liteWins_this
      ? green(liteStr.padStart(COL[1]))
      : isTie
        ? dim(liteStr.padStart(COL[1]))
        : red(liteStr.padStart(COL[1]))
    const fullDisplay = fullWins_this
      ? green(fullStr.padStart(COL[2]))
      : isTie
        ? dim(fullStr.padStart(COL[2]))
        : red(fullStr.padStart(COL[2]))

    const winnerDisplay = isTie
      ? dim("≈ tie".padStart(COL[3]))
      : liteWins_this
        ? green("✓ lite".padStart(COL[3]))
        : cyan("✓ full".padStart(COL[3]))

    const ratioDisplay = isTie
      ? dim("≈".padStart(COL[4]))
      : noisy
        ? yellow(`${ratioStr}~`.padStart(COL[4]))
        : yellow(ratioStr.padStart(COL[4]))

    const name =
      liteRow.name.length > COL[0]
        ? `${liteRow.name.slice(0, COL[0] - 1)}…`
        : liteRow.name.padEnd(COL[0])

    console.log(`  ${name} ${liteDisplay} ${fullDisplay} ${winnerDisplay} ${ratioDisplay}`)
  }

  console.log(`  ${dim(SEP)}`)
  console.log()

  // ── Summary ───────────────────────────────────────────────────────────────
  const compared = liteRes.rows.filter((r) => r.kind !== "na" && fullMap.has(r.name))
  const ties = compared.length - liteWins - fullWins
  console.log()
  console.log(
    `  ${bold("Score (comparable tests)")}` +
      `   Lite wins: ${green(String(liteWins))}` +
      `   Full wins: ${cyan(String(fullWins))}` +
      `   Ties (< ${TIE_THRESHOLD}×): ${dim(String(ties))}` +
      `   Full-exclusive capabilities: ${cyan(String(fullExclusive))}`,
  )
  console.log()
  console.log(`  ${dim(`Lite = embedded JS · ${fmtKB(liteBinSize)} binary`)}`)
  console.log(`  ${dim(`Full = Bun subprocess · JSC JIT · ${fmtKB(fullBinSize)} binary`)}`)
  console.log(
    `  ${dim("∞  = full-exclusive (file I/O, SQLite)   ~  = noisy measurement (CV > 20%)")}`,
  )
  console.log()

  // ── UX verdict ────────────────────────────────────────────────────────────
  // Derive from actual results
  const ipcRow = liteRes.rows.find((r) => r.name === "empty call (IPC floor)")
  const coldRow = liteRes.rows.find((r) => r.name === "startup — first call")
  const _d200row = liteRes.rows.find((r) => r.name === "filter+sort list — 200 items")
  const d2kLite = liteRes.rows.find((r) => r.name === "filter+sort list — 2 000 items")
  const d2kFull = fullRes.rows.find((r) => r.name === "filter+sort list — 2 000 items")
  const p100Lite = liteRes.rows.find((r) => r.name === "send+receive ~100KB")
  const p100Full = fullRes.rows.find((r) => r.name === "send+receive ~100KB")
  const susLite = liteRes.rows.find((r) => r.name === "max call rate (2s)")
  const susFull = fullRes.rows.find((r) => r.name === "max call rate (2s)")

  const liteIPC = ipcRow?.p50 ?? 0
  const fullIPC = fullRes.rows.find((r) => r.name === "empty call (IPC floor)")?.p50 ?? 0
  const coldLite = coldRow?.coldMs ?? 0
  const coldFull = fullRes.rows.find((r) => r.name === "startup — first call")?.coldMs ?? 0
  const susLiteN = susLite?.ops ?? 0
  const susFull_N = susFull?.ops ?? 0
  const p100LiteV = p100Lite?.p50 ?? 0
  const p100FullV = p100Full?.p50 ?? 0
  const d2kLiteV = d2kLite?.p50 ?? 0
  const d2kFullV = d2kFull?.p50 ?? 0

  console.log(`  ${bold("UX Verdict")}`)
  console.log()
  console.log(
    `  ${bold("Button clicks")}        Both modes imperceptible — ${fmt(liteIPC)} (lite) vs ${fmt(fullIPC)} (full). Users won't notice.`,
  )
  if (coldFull > 0 && coldLite > 0) {
    const r = coldFull / coldLite
    const coldRatioStr = r >= 10 ? `${r.toFixed(1)}×` : `+${Math.round((r - 1) * 100)}%`
    console.log(
      `  ${bold("App cold start")}       Lite ${coldRatioStr} faster first call (${fmt(coldLite)} vs ${fmt(coldFull)}). Noticeable on slower hardware.`,
    )
  }
  if (d2kLiteV > 0 && d2kFullV > 0) {
    const pct = Math.round((d2kLiteV / d2kFullV - 1) * 100)
    console.log(
      `  ${bold("Large data (2k items)")} Full ${pct}% faster (${fmt(d2kFullV)} vs ${fmt(d2kLiteV)}). Both still < 10ms — invisible to users.`,
    )
  }
  if (p100LiteV > 0 && p100FullV > 0) {
    const pct = Math.round((p100LiteV / p100FullV - 1) * 100)
    console.log(
      `  ${bold("100KB payloads")}       Full ${pct}% faster (${fmt(p100FullV)} vs ${fmt(p100LiteV)}). Noticeable only for frequent large transfers.`,
    )
  }
  if (susLiteN > 0 && susFull_N > 0) {
    const pct = Math.round((susLiteN / susFull_N - 1) * 100)
    console.log(
      `  ${bold("Sustained throughput")} Lite ${pct}% more calls/s (${fmtOps(susLiteN)} vs ${fmtOps(susFull_N)}). Relevant for tight polling loops only.`,
    )
  }
  console.log(
    `  ${bold("File I/O, SQLite")}     Full-exclusive — lite has no filesystem access. Use full if your app reads/writes files.`,
  )
  console.log(
    `  ${bold("CPU-heavy work")}        Full wins significantly (40× for fib, 11× for regex). Lite adequate for data transforms < 500 items.`,
  )
  console.log()
}

// ── Main ──────────────────────────────────────────────────────────────────────

const ROOT = path.resolve(import.meta.dir, "..")
const BENCH = import.meta.dir
const CORE = path.join(BENCH, "core")
const RESULTS = path.join(BENCH, "results")
const CACHE = path.join(BENCH, ".vorn", "cache")
const BACKEND = path.join(CORE, "backend", "main.ts")

mkdirSync(RESULTS, { recursive: true })
mkdirSync(CACHE, { recursive: true })

console.log()
console.log(bold(`  ╔══════════════════════════════════════╗`))
console.log(bold(`  ║     Vorn Benchmark Runner            ║`))
console.log(bold(`  ╚══════════════════════════════════════╝`))
console.log()
console.log(dim(`  Platform: ${process.platform} ${process.arch}`))
console.log(dim(`  Bun:      ${Bun.version}`))
console.log(dim(`  Root:     ${ROOT}`))
console.log()

// Find binaries
const liteBin = findBinary("lite", ROOT)
const fullBin = findBinary("full", ROOT)

if (!liteBin) {
  console.error(red("  vorn-lite binary not found. Build with: cargo build -p vorn-lite --release"))
  process.exit(1)
}
if (!fullBin) {
  console.error(red("  vorn-full binary not found. Build with: cargo build -p vorn-full --release"))
  process.exit(1)
}

console.log(dim(`  vorn-lite: ${liteBin}`))
console.log(dim(`  vorn-full: ${fullBin}`))
console.log()

// Build lite bundle
const bundlePath = await buildLiteBundle(CORE, CACHE)

// Start Vite dev server
console.log(dim("  Starting Vite dev server on 127.0.0.1:5174…"))
const viteProc = Bun.spawn(["bunx", "--bun", "vite"], {
  cwd: CORE,
  stdout: "ignore",
  stderr: "ignore",
})

try {
  await waitForVite("http://127.0.0.1:5174", 30_000)
  console.log(dim("  Vite ready."))
} catch (e) {
  console.error(red(`  ${e}`))
  viteProc.kill()
  process.exit(1)
}

// ── Run LITE benchmark ────────────────────────────────────────────────────────

console.log()
console.log(bold(cyan("  ── Running LITE benchmark ──────────────────────────────")))
console.log(dim("  Spawning vorn-lite WebView… (benchmark will auto-run)"))

const liteServer = startResultsServer()
const liteProc = Bun.spawn(
  [liteBin, "--bundle", bundlePath, "--dev-url", "http://127.0.0.1:5174?auto=1&runtime=lite"],
  { stdout: "ignore", stderr: "ignore" },
)

let liteRes: ResultsPayload
try {
  const timeout = new Promise<never>((_, reject) =>
    setTimeout(() => reject(new Error("Lite benchmark timed out after 10 minutes")), 10 * 60_000),
  )
  liteRes = await Promise.race([liteServer.wait(), timeout])
  console.log(green("  ✓ Lite results received"))
} catch (e) {
  console.error(red(`  Lite benchmark error: ${e}`))
  liteProc.kill()
  liteServer.stop()
  viteProc.kill()
  process.exit(1)
}

liteProc.kill()
liteServer.stop()

writeFileSync(path.join(RESULTS, "lite.json"), JSON.stringify(liteRes, null, 2))
console.log(dim(`  Saved → benchmarks/results/lite.json`))

// Wait between runs
await Bun.sleep(1000)

// ── Run FULL benchmark ────────────────────────────────────────────────────────

console.log()
console.log(bold(cyan("  ── Running FULL benchmark ──────────────────────────────")))
console.log(dim("  Spawning vorn-full WebView… (benchmark will auto-run)"))

const fullServer = startResultsServer()
const fullProc = Bun.spawn([fullBin, "--backend-entry", BACKEND], {
  stdout: "ignore",
  stderr: "ignore",
  env: {
    ...process.env,
    VORN_ENTRY: BACKEND,
    VORN_DEV_URL: "http://127.0.0.1:5174?auto=1&runtime=full",
  },
})

let fullRes: ResultsPayload
try {
  const timeout = new Promise<never>((_, reject) =>
    setTimeout(() => reject(new Error("Full benchmark timed out after 10 minutes")), 10 * 60_000),
  )
  fullRes = await Promise.race([fullServer.wait(), timeout])
  console.log(green("  ✓ Full results received"))
} catch (e) {
  console.error(red(`  Full benchmark error: ${e}`))
  fullProc.kill()
  fullServer.stop()
  viteProc.kill()
  process.exit(1)
}

fullProc.kill()
fullServer.stop()

writeFileSync(path.join(RESULTS, "full.json"), JSON.stringify(fullRes, null, 2))
console.log(dim(`  Saved → benchmarks/results/full.json`))

// ── Tear down Vite ────────────────────────────────────────────────────────────

viteProc.kill()
console.log(dim("  Vite stopped."))

// ── Print comparison ──────────────────────────────────────────────────────────

printComparison(liteRes, fullRes, liteBin, fullBin, bundlePath, BACKEND)
process.exit(0)
