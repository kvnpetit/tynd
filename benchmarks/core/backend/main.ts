import { app, createEmitter } from "@vorn/core"

export const events = createEmitter<{
  "bench:pong": { seq: number }
}>()

// `typeof Bun !== "undefined"` is evaluated at build time by Bun.build and becomes `true`
// in the lite bundle. Use process.platform instead — defined as "quickjs" via `define` at build time.
const IS_FULL = process.platform !== "quickjs"

// String refs prevent bundler static analysis from pulling in Bun/Node modules
// when building the lite bundle (target: "browser")
const _os     = "os"
const _path   = "path"
const _sqlite = "bun:sqlite"

export function getCapabilities(): { fileIO: boolean; sqlite: boolean; runtime: "full" | "lite" } {
  return { fileIO: IS_FULL, sqlite: IS_FULL, runtime: IS_FULL ? "full" : "lite" }
}

export function ping(): "pong" {
  return "pong"
}

export function echoPayload(data: unknown): unknown {
  return data
}

export function generatePayload(kb: number): unknown[] {
  const count = Math.ceil((kb * 1024) / 50)
  return Array.from({ length: count }, (_, i) => ({
    id:    i,
    name:  `item-${i}`,
    value: Math.PI * i,
    flag:  i % 2 === 0,
  }))
}

export function deepObject(depth: number): unknown {
  let obj: unknown = { leaf: true, value: Math.PI }
  for (let i = 0; i < depth; i++) {
    obj = { level: i, child: obj, tag: `node-${i}`, v: Math.random() }
  }
  return obj
}

// ── Realistic app scenarios (both modes) ──────────────────────────────────────
// Uses Math.sin for deterministic output (not Math.random) so results are reproducible.

export function processDataset(size: number): { count: number; avg: number; topScore: number } {
  const records = Array.from({ length: size }, (_, i) => ({
    id:     i,
    score:  Math.sin(i) * 50 + 50,
    active: i % 3 !== 0,
    tags:   [`tag-${i % 10}`, `cat-${i % 5}`],
  }))
  const active = records.filter(r => r.active && r.score > 20)
  active.sort((a, b) => b.score - a.score)
  const avg = active.reduce((s, r) => s + r.score, 0) / (active.length || 1)
  return { count: active.length, avg, topScore: active[0]?.score ?? 0 }
}

export function processApiResponse(n: number): { valid: number; invalid: number; total: number } {
  const items = Array.from({ length: n }, (_, i) => ({
    status: i % 7 === 0 ? "error" : "ok",
    data:   { userId: i, email: `user${i}@example.com`, score: (i * 17) % 100 },
    ts:     1_700_000_000_000 - i * 1000,
  }))
  let valid = 0, invalid = 0
  for (const r of items) {
    if (r.status === "ok" && r.data.email.includes("@") && r.data.score >= 0) valid++
    else invalid++
  }
  return { valid, invalid, total: n }
}

export function buildReport(rows: number): number {
  const data = Array.from({ length: rows }, (_, i) => ({
    name:  `Item-${String(i).padStart(4, "0")}`,
    value: (i * 137) % 10_000,
    pct:   (i * 31) % 100,
  }))
  const total = data.reduce((s, d) => s + d.value, 0)
  let out = `Report — ${rows} items\nTotal: ${total.toFixed(2)}\n\n`
  for (const d of data) {
    out += `  ${d.name.padEnd(12)} ${d.value.toFixed(2).padStart(10)}  (${d.pct.toFixed(1)}%)\n`
  }
  return out.length
}

// ── File I/O — full only ──────────────────────────────────────────────────────

const _tmpFiles: Record<string, string> = {}

export async function setupFileIO(): Promise<boolean> {
  if (!IS_FULL) return false
  const { tmpdir } = await import(_os)   as typeof import("os")
  const { join }   = await import(_path) as typeof import("path")
  for (const [label, kb] of [["1kb", 1], ["100kb", 100], ["1mb", 1024]] as const) {
    const p = join(tmpdir(), `vorn-bench-${label}.dat`)
    await Bun.write(p, Buffer.alloc(kb * 1024, 0x41))
    _tmpFiles[label] = p
  }
  return true
}

export async function fileRead(sizeLabel: "1kb" | "100kb" | "1mb"): Promise<number | null> {
  if (!IS_FULL) return null
  const p = _tmpFiles[sizeLabel]
  if (!p) return null
  const buf = await Bun.file(p).arrayBuffer()
  return buf.byteLength
}

export async function fileWrite(kb: number): Promise<number | null> {
  if (!IS_FULL) return null
  const { tmpdir } = await import(_os)   as typeof import("os")
  const { join }   = await import(_path) as typeof import("path")
  const buf = Buffer.alloc(kb * 1024, 0x42)
  await Bun.write(join(tmpdir(), "vorn-bench-write.dat"), buf)
  return buf.byteLength
}

export async function fileRoundtrip(kb: number): Promise<number | null> {
  if (!IS_FULL) return null
  const { tmpdir } = await import(_os)   as typeof import("os")
  const { join }   = await import(_path) as typeof import("path")
  const p = join(tmpdir(), "vorn-bench-rtrip.dat")
  const buf = Buffer.alloc(kb * 1024, 0x43)
  await Bun.write(p, buf)
  const back = await Bun.file(p).arrayBuffer()
  return back.byteLength
}

// ── SQLite — full only ────────────────────────────────────────────────────────

interface DB {
  run(sql: string): void
  prepare(sql: string): {
    get(...args: unknown[]): unknown
    all(...args: unknown[]): unknown[]
    run(...args: unknown[]): void
  }
  transaction(fn: () => void): () => void
}

let _db: DB | null = null

export async function setupSQLite(): Promise<boolean> {
  if (!IS_FULL) return false
  const { Database } = await import(_sqlite) as typeof import("bun:sqlite")
  const db = new Database(":memory:") as unknown as DB
  db.run(`CREATE TABLE items (
    id       INTEGER PRIMARY KEY,
    name     TEXT    NOT NULL,
    score    REAL    NOT NULL,
    category TEXT    NOT NULL,
    active   INTEGER NOT NULL
  )`)
  const cats   = ["alpha", "beta", "gamma", "delta", "epsilon"]
  const insert = db.prepare("INSERT INTO items VALUES (?, ?, ?, ?, ?)")
  db.transaction(() => {
    for (let i = 0; i < 10_000; i++) {
      insert.run(i, `item-${i}`, (i * 17) % 100, cats[i % 5]!, i % 3 !== 0 ? 1 : 0)
    }
  })()
  _db = db
  return true
}

export function sqliteSelectPk(id: number): unknown | null {
  if (!IS_FULL || !_db) return null
  return _db.prepare("SELECT * FROM items WHERE id = ?").get(id)
}

export function sqliteSelectRange(n: number): number | null {
  if (!IS_FULL || !_db) return null
  return (_db.prepare("SELECT * FROM items ORDER BY score DESC LIMIT ?").all(n)).length
}

export function sqliteInsertBatch(n: number): number | null {
  if (!IS_FULL || !_db) return null
  const insert = _db.prepare("INSERT OR REPLACE INTO items VALUES (?, ?, ?, ?, ?)")
  const base   = 200_000
  _db.transaction(() => {
    for (let i = 0; i < n; i++) {
      insert.run(base + i, `batch-${i}`, (i * 13) % 100, "test", 1)
    }
  })()
  return n
}

export function sqliteAggregate(): unknown[] | null {
  if (!IS_FULL || !_db) return null
  return _db.prepare(
    "SELECT category, COUNT(*) as cnt, AVG(score) as avg_score FROM items GROUP BY category"
  ).all()
}

export function sqliteSearch(term: string): number | null {
  if (!IS_FULL || !_db) return null
  return (_db.prepare("SELECT id FROM items WHERE name LIKE ?").all(`%${term}%`)).length
}

// ── CPU — interpreter vs JIT ──────────────────────────────────────────────────

export function fibonacci(n: number): number {
  if (n <= 1) return n
  return fibonacci(n - 1) + fibonacci(n - 2)
}

export function sortInPlace(n: number): number {
  const arr = Array.from({ length: n }, () => Math.random())
  arr.sort((a, b) => a - b)
  return arr[0]!
}

export function stringWork(n: number): number {
  let s = ""
  for (let i = 0; i < n; i++) s += "x"
  return s.length
}

export function arrayTransform(n: number): number {
  return Array.from({ length: n }, (_, i) => i)
    .map(x => x * 2 + 1)
    .filter(x => x % 3 !== 0)
    .reduce((acc, x) => acc + x, 0)
}

export function jsonWork(n: number): number {
  const obj = {
    id:     42,
    name:   "benchmark",
    values: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
    nested: { a: true, b: "hello", c: Math.PI },
    tags:   ["alpha", "beta", "gamma"],
  }
  let len = 0
  for (let i = 0; i < n; i++) {
    len = JSON.parse(JSON.stringify(obj)).id
  }
  return len
}

export function regexWork(n: number): number {
  const text = "The quick brown fox jumps over the lazy dog. ".repeat(20)
  const re   = /\b\w{4,6}\b/g
  let count  = 0
  for (let i = 0; i < n; i++) {
    count = (text.match(re) ?? []).length
  }
  return count
}

export function objectAlloc(n: number): number {
  const items = []
  for (let i = 0; i < n; i++) {
    items.push({ id: i, x: Math.random(), y: Math.random(), label: `p${i}` })
  }
  return items.length
}

// ── Memory ────────────────────────────────────────────────────────────────────

export function getMemoryUsage(): { heapUsedMB: number; rssMB: number } {
  try {
    const m = (process as NodeJS.Process).memoryUsage()
    return { heapUsedMB: m.heapUsed / (1024 * 1024), rssMB: m.rss / (1024 * 1024) }
  } catch {
    return { heapUsedMB: 0, rssMB: 0 }
  }
}

// ── Event push ────────────────────────────────────────────────────────────────

export function emitPong(seq: number): void {
  events.emit("bench:pong", { seq })
}

app.start({
  window: {
    title:  "IPC Benchmark",
    width:  960,
    height: 820,
    center: true,
  },
})
