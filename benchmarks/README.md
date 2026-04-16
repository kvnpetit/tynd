# Vorn Benchmarks

Benchmark suite comparing **lite** vs **full** runtimes across every axis that matters for a desktop app.

---

## Prerequisites

### 1. Rust toolchain

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### 2. Bun

```bash
curl -fsSL https://bun.sh/install | bash
```

### 3. Build the Vorn binaries

```bash
# From repo root
cargo build -p vorn-lite --release
cargo build -p vorn-full --release
```

### 4. Install JS dependencies

```bash
bun install
```

---

## Run

```bash
bun bench
```

Opens two WebView windows in sequence (lite, then full), runs all benchmarks automatically, and prints a side-by-side comparison table.

---

## What is measured

### App Startup

Cold measurements — no warmup. This is what users feel at launch.

| Test | What it measures |
|------|-----------------|
| `startup — first call` | Time to first backend response |
| `startup — load initial data (100)` | First screen population |
| `startup — parse config (JSON ×10)` | Config parsing at launch |
| `startup — first live event` | Time to first real-time notification |

Lite: 0.5ms first call. Full: 0.7ms. Both imperceptible.

### Button Click — Dispatch Floor

Minimum latency every backend call pays, after warmup.

| Test | What it measures |
|------|-----------------|
| `empty call (IPC floor)` | Raw round-trip from UI to backend and back |

Both modes: ~0.3ms. Invisible to users (human reaction time is ~100–250ms).

### Typical App Workloads

Realistic backend operations — most relevant for choosing a runtime.

| Test | What it measures |
|------|-----------------|
| `filter+sort list — 200 / 2 000 items` | Loading a table, sidebar, or search results |
| `validate API batch — 50 / 500 items` | Processing a form or API response |
| `build report — 50 / 500 rows` | Generating a formatted export |

Full wins at all sizes. Both stay under 5ms — invisible to users.

### File Operations — full only

Lite shows N/A — no filesystem access in lite mode.

| Test | What it measures |
|------|-----------------|
| `read config — 1KB` | Loading app settings |
| `read document — 100KB` | Opening a medium file |
| `read large file — 1MB` | Loading a large document |
| `save settings — 1KB` | Writing user preferences |
| `settings roundtrip — write+read 1KB` | Save + verify |

### Local Database — full only

Lite shows N/A — no SQLite in lite mode.

| Test | What it measures |
|------|-----------------|
| `db lookup by ID` | Autocomplete, detail view |
| `db list top 100 results` | Search results, paginated list |
| `db save 100 records (batch)` | Bulk insert, log flush |
| `db dashboard aggregate` | GROUP BY stats |
| `db full-text search (LIKE)` | Search across 10k rows |

### Data Transfer

Cost of moving data between backend and UI.

| Test | What it measures |
|------|-----------------|
| `send+receive 1KB / 10KB / 100KB` | Round-trip payloads |
| `backend → UI list ~10KB / 100KB` | Backend pushing data to frontend |
| `nested config object (depth 10)` | Deep nested structures |

### Real-time Updates

Backend pushes an event; frontend measures time to handler.

| Test | What it measures |
|------|-----------------|
| `backend event → UI handler` | Notification, progress bar, live feed |

Both modes: ~0.3ms. Tie.

### Concurrent Requests

All N calls fired simultaneously. Tests parallel throughput.

| Test | What it measures |
|------|-----------------|
| `10 / 50 / 100 simultaneous calls` | Dashboard with multiple panels |
| `10 / 50 simultaneous calls with 1KB payload` | Parallel requests with data |

Lite wins — +100–300% faster per-call at high concurrency.

### Sustained Throughput

Runs for 2 seconds, counts total calls.

| Test | What it measures |
|------|-----------------|
| `max call rate (2s)` | Polling loop, real-time streaming |
| `max call rate with small payload` | Throughput with minimal serialization |
| `max JSON ops/s (2s)` | Sustained JSON processing |

Lite: 4 200/s. Full: 3 200/s.

### Compute Benchmarks (Synthetic)

Pure CPU stress tests. Real apps rarely hit these extremes.

| Test | What it measures |
|------|-----------------|
| `fibonacci(25/30)` | Recursive computation |
| `sort 1k / 10k numbers` | Array sort |
| `map→filter→reduce 1k / 10k` | Functional pipeline |
| `JSON stringify+parse ×100` | JSON throughput |
| `regex match ×100` | Regex engine |
| `string concat ×10k` | String allocation |
| `object alloc ×1k / 10k` | Object creation |

Full wins heavily here (5–53×). For real workloads under 2 000 items, both modes stay under 5ms.

---

## Results

### What users actually feel

Measured on Windows 11, Bun 1.3.11, release binaries.

| Situation | Lite | Full | Visible to user? |
|-----------|------|------|-----------------|
| Button click (warmed) | 0.3 ms | 0.3 ms | No |
| Cold start — first call | 0.5 ms | 0.7 ms | No |
| Cold start — load 100 items | 0.5 ms | 1.7 ms | No |
| Load 200 items | 0.8 ms | 0.3 ms | No |
| Load 2 000 items | 3.8 ms | 0.6 ms | No |
| Read 1MB file | N/A | 0.6 ms | No |
| SQLite aggregate | N/A | 1.6 ms | No |
| Sustained call rate | 4 200/s | 3 200/s | Only in tight loops |

Both modes feel instant for typical interactions.

### Key numbers

| Metric | Lite | Full |
|--------|------|------|
| IPC floor (warmed) | 0.3 ms | 0.3 ms |
| Cold start first call | 0.5 ms | 0.7 ms |
| Filter+sort 2 000 items | 3.8 ms | 0.6 ms |
| 100 concurrent calls | 0.031 ms/call | 0.069 ms/call |
| Sustained call rate | 4 200/s | 3 200/s |
| Send+receive 100 KB | 8.6 ms | 3.6 ms |
| Read 1MB file | N/A | 0.6 ms |
| SQLite aggregate | N/A | 1.6 ms |

---

## Raw results

After `bun bench`, results are saved to:

```
benchmarks/results/lite.json
benchmarks/results/full.json
```

---

## Architecture

```
benchmarks/
├── run.ts           ← Launcher: spawns binaries, collects results, prints comparison
├── core/
│   ├── backend/
│   │   └── main.ts  ← Backend functions (both modes use the same source)
│   └── src/
│       └── main.ts  ← WebView frontend: runs all sections, POSTs results
└── results/         ← JSON output after bun bench
```
