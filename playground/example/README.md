# playground/example

Minimal reference app for Tynd's **`lite`** runtime — a tiny RPC + event demo that fits on one screen. Companion to [`playground/full`](../full/README.md) (which ships a real LLM chatbot).

```bash
# From the repo root
cargo build --release -p tynd-lite          # one-time: build the host
bun install

# Then from this directory
bun run dev          # tynd dev — HMR frontend + hot-reload backend bundle
bun run build        # tynd build — single ~6.6 MB binary in release/
bun run start        # tynd start — run cached bundles, no rebuild
```

## Why lite?

- **~6.5 MB** host, ~6.6 MB final binary with this playground's assets packed in.
- No external runtime — backend runs in-process.
- Fastest cold start.
- Same OS APIs as `full`: `fs`, `http`, `websocket`, `sql`, `compute`, `terminal`, and the rest.

When **not** to pick lite — if you need Bun's JIT for hot CPU-bound JS, JS-level `fetch` / `Intl` / `Buffer` in backend code, or native-binding npm packages. See [RUNTIMES.md](../../RUNTIMES.md).

## Layout

```
playground/example/
├── backend/main.ts       ← exported RPC fns + app.start()
├── src/                  ← React + Vite frontend
├── tynd.config.ts        ← runtime: "lite"
└── vite.config.ts
```

## Running against a local host build

The CLI prefers `target/release` inside the workspace. Rebuild the lite host with `cargo build --release -p tynd-lite` and the next `tynd dev` / `tynd start` picks it up.
