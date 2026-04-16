import { createBackend } from "@vorn/core/client"
import type * as backend from "../backend/main"

const api = createBackend<typeof backend>()

type BenchRow = {
  name:     string
  kind:     "cold" | "latency" | "concurrent" | "sustained" | "na"
  p50?:     number
  p95?:     number
  p99?:     number
  min?:     number
  max?:     number
  stddev?:  number
  cv?:      number
  ops?:     number
  coldMs?:  number
  perCall?: number
  total?:   number
  calls?:   number
}

type ResultsPayload = {
  runtime:         string
  platform:        string
  timestamp:       string
  rows:            BenchRow[]
  frontendHeapMB?: number
  backendHeapMB?:  number
  backendRssMB?:   number
}

function stats(samples: number[]) {
  const s    = [...samples].sort((a, b) => a - b)
  const pct  = (p: number) => s[Math.min(Math.ceil((p / 100) * s.length), s.length - 1)]!
  const mean = s.reduce((a, b) => a + b, 0) / s.length
  const variance = s.reduce((acc, v) => acc + (v - mean) ** 2, 0) / s.length
  const stddev   = Math.sqrt(variance)
  return {
    min:    s[0]!,
    p50:    pct(50),
    p95:    pct(95),
    p99:    pct(99),
    max:    s[s.length - 1]!,
    mean,
    stddev,
    cv:     mean > 0 ? (stddev / mean) * 100 : 0,
    ops:    Math.round(1000 / pct(50)),
  }
}

async function bench(
  fn:     () => Promise<unknown>,
  warmup: number,
  iters:  number,
): Promise<ReturnType<typeof stats>> {
  for (let i = 0; i < warmup; i++) await fn()
  const times: number[] = []
  for (let i = 0; i < iters; i++) {
    const t0 = performance.now()
    await fn()
    times.push(performance.now() - t0)
  }
  return stats(times)
}

async function cold(fn: () => Promise<unknown>): Promise<number> {
  const t0 = performance.now()
  await fn()
  return performance.now() - t0
}

async function benchConcurrent(
  fn:    () => Promise<unknown>,
  count: number,
): Promise<{ total: number; perCall: number; ops: number }> {
  const t0    = performance.now()
  await Promise.all(Array.from({ length: count }, () => fn()))
  const total = performance.now() - t0
  return { total, perCall: total / count, ops: Math.round(count / (total / 1000)) }
}

async function benchSustained(
  fn: () => Promise<unknown>,
  ms: number,
): Promise<{ ops: number; calls: number }> {
  const deadline = performance.now() + ms
  let calls = 0
  while (performance.now() < deadline) { await fn(); calls++ }
  return { ops: Math.round(calls / (ms / 1000)), calls }
}

async function benchEventPush(warmup: number, iters: number): Promise<ReturnType<typeof stats>> {
  const times: number[] = []
  const runOne = (seq: number, record: boolean): Promise<void> =>
    new Promise(resolve => {
      const unsub = api.on("bench:pong", () => {
        if (record) times.push(performance.now() - t0)
        unsub(); resolve()
      })
      const t0 = performance.now()
      void api.emitPong(seq)
    })
  for (let i = 0; i < warmup; i++) await runOne(i, false)
  for (let i = 0; i < iters; i++) await runOne(warmup + i, true)
  return stats(times)
}

const $ = (sel: string) => document.querySelector(sel) as HTMLElement

function setStatus(msg: string, done = false) {
  const el = $(".status")
  el.textContent = msg
  el.className = done ? "status done" : "status"
}

function log(text: string, na = false) {
  const li = document.createElement("li")
  if (na) li.className = "na"
  li.textContent = text
  $(".results-log").appendChild(li)
}

function pushLatency(rows: BenchRow[], name: string, s: ReturnType<typeof stats>) {
  rows.push({ name, kind: "latency", p50: s.p50, p95: s.p95, p99: s.p99, min: s.min, max: s.max, ops: s.ops, stddev: s.stddev, cv: s.cv })
}

function logLatency(rows: BenchRow[], name: string, s: ReturnType<typeof stats>) {
  pushLatency(rows, name, s)
  log(`${name}  P50 ${s.p50.toFixed(3)}ms  ${s.ops.toLocaleString()} ops/s`)
}

function logCold(rows: BenchRow[], name: string, ms: number) {
  rows.push({ name, kind: "cold", coldMs: ms })
  log(`${name}  ${ms.toFixed(2)}ms  (cold)`)
}

function logNA(rows: BenchRow[], name: string) {
  rows.push({ name, kind: "na" })
  log(`${name}  N/A (lite mode — no Node.js APIs)`, true)
}

function logConcurrent(rows: BenchRow[], name: string, r: { total: number; perCall: number; ops: number }) {
  rows.push({ name, kind: "concurrent", perCall: r.perCall, total: r.total, ops: r.ops })
  log(`${name}  perCall ${r.perCall.toFixed(3)}ms  total ${r.total.toFixed(1)}ms`)
}

function logSustained(rows: BenchRow[], name: string, r: { ops: number; calls: number }) {
  rows.push({ name, kind: "sustained", ops: r.ops, calls: r.calls })
  log(`${name}  ${r.ops.toLocaleString()} ops/s  (${r.calls} calls in 2s)`)
}

async function runBenchmark(): Promise<void> {
  const btn = $(".run-btn") as HTMLButtonElement
  btn.disabled = true
  $(".results-log").innerHTML = ""
  setStatus("Starting…")

  const rows: BenchRow[] = []

  let peakFrontendHeap = 0
  const perfWithMemory = performance as Performance & { memory?: { usedJSHeapSize: number } }
  const heapInterval = setInterval(() => {
    const used = perfWithMemory.memory?.usedJSHeapSize ?? 0
    if (used > peakFrontendHeap) peakFrontendHeap = used
  }, 200)

  try {

    // Fetch capabilities once — used to decide N/A vs real bench for file IO and SQLite
    const caps = await api.getCapabilities()

    // ① App startup — cold, no warmup
    setStatus("① App startup (cold)…")
    logCold(rows, "startup — first call",              await cold(() => api.ping()))
    logCold(rows, "startup — load initial data (100)", await cold(() => api.processDataset(100)))
    logCold(rows, "startup — parse config (JSON ×10)", await cold(() => api.jsonWork(10)))
    logCold(rows, "startup — first live event",        await cold(() =>
      new Promise<void>(resolve => {
        const unsub = api.on("bench:pong", () => { unsub(); resolve() })
        void api.emitPong(0)
      })
    ))

    // ② Button click — dispatch floor (warmed)
    setStatus("② Button click — dispatch floor…")
    const ping = await bench(() => api.ping(), 20, 500)
    logLatency(rows, "empty call (IPC floor)",         ping)
    logLatency(rows, "empty call — with result check", await bench(() => api.ping().then(r => r === "pong"), 10, 200))

    // ③ Typical app workloads — same logic in both modes (embedded vs JIT)
    setStatus("③ Typical app workloads…")
    logLatency(rows, "filter+sort list — 200 items",   await bench(() => api.processDataset(200),        5, 50))
    logLatency(rows, "filter+sort list — 2 000 items", await bench(() => api.processDataset(2_000),       5, 30))
    logLatency(rows, "validate API batch — 50 items",  await bench(() => api.processApiResponse(50),      5, 100))
    logLatency(rows, "validate API batch — 500 items", await bench(() => api.processApiResponse(500),     5, 50))
    logLatency(rows, "build report — 50 rows",         await bench(() => api.buildReport(50),             5, 50))
    logLatency(rows, "build report — 500 rows",        await bench(() => api.buildReport(500),            5, 20))

    // ④ File operations — full only
    setStatus("④ File operations…")
    if (caps.fileIO) await api.setupFileIO()
    if (!caps.fileIO) {
      logNA(rows, "read config — 1KB")
      logNA(rows, "read document — 100KB")
      logNA(rows, "read large file — 1MB")
      logNA(rows, "save settings — 1KB")
      logNA(rows, "settings roundtrip — write+read 1KB")
    } else {
      logLatency(rows, "read config — 1KB",                   await bench(() => api.fileRead("1kb"),   3, 50))
      logLatency(rows, "read document — 100KB",               await bench(() => api.fileRead("100kb"), 3, 30))
      logLatency(rows, "read large file — 1MB",               await bench(() => api.fileRead("1mb"),   3, 20))
      logLatency(rows, "save settings — 1KB",                 await bench(() => api.fileWrite(1),      3, 30))
      logLatency(rows, "settings roundtrip — write+read 1KB", await bench(() => api.fileRoundtrip(1),  3, 30))
    }

    // ⑤ Local database — full only
    setStatus("⑤ Local database (SQLite)…")
    if (caps.sqlite) await api.setupSQLite()
    if (!caps.sqlite) {
      logNA(rows, "db lookup by ID")
      logNA(rows, "db list top 100 results")
      logNA(rows, "db save 100 records (batch)")
      logNA(rows, "db dashboard aggregate")
      logNA(rows, "db full-text search (LIKE)")
    } else {
      logLatency(rows, "db lookup by ID",             await bench(() => api.sqliteSelectPk(42),     5, 200))
      logLatency(rows, "db list top 100 results",     await bench(() => api.sqliteSelectRange(100), 5, 100))
      logLatency(rows, "db save 100 records (batch)", await bench(() => api.sqliteInsertBatch(100), 3, 30))
      logLatency(rows, "db dashboard aggregate",      await bench(() => api.sqliteAggregate(),      5, 100))
      logLatency(rows, "db full-text search (LIKE)",  await bench(() => api.sqliteSearch("item-1"), 5, 100))
    }

    // ⑥ Data transfer to/from UI
    setStatus("⑥ Data transfer to/from UI…")
    logLatency(rows, "send+receive 1KB",                await bench(() => api.echoPayload({ data: "x".repeat(900) }),                                     5, 100))
    logLatency(rows, "send+receive ~10KB",              await bench(() => api.echoPayload(Array.from({ length: 200 },  (_, i) => ({ id: i, v: Math.PI }))), 5, 50))
    logLatency(rows, "send+receive ~100KB",             await bench(() => api.echoPayload(Array.from({ length: 2000 }, (_, i) => ({ id: i, v: Math.PI }))), 3, 20))
    logLatency(rows, "backend → UI list ~10KB",         await bench(() => api.generatePayload(10),  5, 50))
    logLatency(rows, "backend → UI list ~100KB",        await bench(() => api.generatePayload(100), 3, 20))
    logLatency(rows, "nested config object (depth 10)", await bench(() => api.deepObject(10),       5, 100))

    // ⑦ Real-time updates — event push pipeline
    setStatus("⑦ Real-time updates (event push)…")
    logLatency(rows, "backend event → UI handler", await benchEventPush(10, 200))

    // ⑧ Concurrent requests
    setStatus("⑧ Concurrent requests…")
    for (const n of [10, 50, 100] as const) {
      logConcurrent(rows, `${n} simultaneous calls`, await benchConcurrent(() => api.ping(), n))
    }
    for (const n of [10, 50] as const) {
      const payload = { data: "x".repeat(900) }
      logConcurrent(rows, `${n} simultaneous calls with 1KB payload`, await benchConcurrent(() => api.echoPayload(payload), n))
    }

    // ⑨ Sustained throughput (2s)
    setStatus("⑨ Sustained throughput (2s each)…")
    logSustained(rows, "max call rate (2s)",               await benchSustained(() => api.ping(), 2000))
    logSustained(rows, "max call rate with small payload", await benchSustained(() => api.echoPayload({ x: 42 }), 2000))
    logSustained(rows, "max JSON ops/s (2s)",              await benchSustained(() => api.jsonWork(10), 2000))

    // ⑩ JS engine — synthetic stress tests (not typical app workloads)
    setStatus("⑩ JS engine benchmarks (synthetic)…")
    logLatency(rows, "[engine] fibonacci(25) — recursive",       await bench(() => api.fibonacci(25),          3, 20))
    logLatency(rows, "[engine] fibonacci(30) — recursive",       await bench(() => api.fibonacci(30),          2,  5))
    logLatency(rows, "[engine] sort 1 000 numbers",              await bench(() => api.sortInPlace(1_000),     5, 50))
    logLatency(rows, "[engine] sort 10 000 numbers",             await bench(() => api.sortInPlace(10_000),    5, 20))
    logLatency(rows, "[engine] map→filter→reduce 1 000 items",   await bench(() => api.arrayTransform(1_000),  5, 50))
    logLatency(rows, "[engine] map→filter→reduce 10 000 items",  await bench(() => api.arrayTransform(10_000), 5, 20))
    logLatency(rows, "[engine] JSON stringify+parse ×100",       await bench(() => api.jsonWork(100),          5, 50))
    logLatency(rows, "[engine] regex match ×100",                await bench(() => api.regexWork(100),         5, 50))
    logLatency(rows, "[engine] string concat ×10 000",           await bench(() => api.stringWork(10_000),     5, 30))
    logLatency(rows, "[engine] object alloc ×1 000",             await bench(() => api.objectAlloc(1_000),     5, 50))
    logLatency(rows, "[engine] object alloc ×10 000",            await bench(() => api.objectAlloc(10_000),    5, 20))

    setStatus(
      `✓ Done — dispatch floor P50 ${ping.p50.toFixed(3)}ms · ${ping.ops.toLocaleString()} calls/sec`,
      true,
    )

    clearInterval(heapInterval)
    const frontendHeapMB = peakFrontendHeap > 0 ? peakFrontendHeap / (1024 * 1024) : undefined

    let backendHeapMB: number | undefined
    let backendRssMB:  number | undefined
    try {
      const mem = await api.getMemoryUsage()
      if (mem.rssMB > 0) { backendHeapMB = mem.heapUsedMB; backendRssMB = mem.rssMB }
    } catch { /* lite — not available */ }

    if (AUTO) {
      const payload: ResultsPayload = {
        runtime:   RUNTIME,
        platform:  navigator.platform,
        timestamp: new Date().toISOString(),
        rows,
        frontendHeapMB,
        backendHeapMB,
        backendRssMB,
      }
      try {
        await fetch("http://127.0.0.1:9876/results", {
          method:  "POST",
          headers: { "Content-Type": "application/json" },
          body:    JSON.stringify(payload),
        })
        setStatus("✓ Results sent to terminal. You can close this window.", true)
      } catch {
        setStatus("Could not send results to terminal (is run.ts running?)")
      }
    }

  } catch (e) {
    clearInterval(heapInterval)
    setStatus(`Error: ${String(e)}`)
  }

  btn.disabled = false
}

const _params        = new URLSearchParams(window.location.search)
const AUTO           = _params.has("auto")
const _urlRuntime    = _params.get("runtime")
const _g             = globalThis as Record<string, unknown>
const _defineRuntime = (_g["__VORN_RUNTIME__"] as string | undefined) ?? "dev"
const RUNTIME: string = (_urlRuntime && _urlRuntime !== "dev") ? _urlRuntime : _defineRuntime

document.addEventListener("DOMContentLoaded", () => {
  const badge = $(".runtime-badge")
  if (RUNTIME === "dev") {
    badge.textContent = "dev mode"
    badge.classList.add("badge-dev")
  } else {
    badge.textContent = RUNTIME === "lite" ? "lite" : "full"
    badge.classList.add(RUNTIME === "lite" ? "badge-lite" : "badge-full")
  }

  if (AUTO) {
    $(".run-btn").style.display = "none"
    setStatus("Auto-run — starting in 300ms…")
    setTimeout(() => { void runBenchmark() }, 300)
  } else {
    $(".run-btn").addEventListener("click", () => { void runBenchmark() })
  }
})
