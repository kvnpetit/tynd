# playground/full

Reference app for Tynd's **`full`** runtime (Bun subprocess). Exercises the backend RPC + every OS API the frontend can reach.

```bash
# From the repo root
cargo build --release -p tynd-full          # one-time: build the host
bun install

# Then from this directory
bun run dev          # tynd dev — HMR frontend + hot-reload backend
bun run build        # tynd build — single .exe / binary in release/
bun run start        # tynd start — run cached bundles, no rebuild
```

## What this demonstrates

- **Typed RPC** — `backend/main.ts` exports functions consumed via `createBackend<typeof backend>()` in `src/`.
- **Every OS API** — `fs`, `dialog`, `clipboard`, `shell`, `notification`, `process`, `http`, `websocket`, `sql`, `terminal`, `compute`, `tray`, `tyndWindow`, etc. Each has a button in the UI.
- **Lifecycle hooks** — `app.onReady` / `app.onClose` wired up in the backend.
- **Tray + menu** — custom items with click handlers.

## Layout

```
playground/full/
├── backend/main.ts       ← exported RPC fns + app.start()
├── src/                  ← React + Vite frontend
│   ├── App.tsx
│   └── panels/           ← one panel per OS API
├── tynd.config.ts        ← runtime: "full"
└── vite.config.ts
```

## Running against a local host build

The Tynd CLI's `findBinary` prefers `target/release` inside the workspace. If you rebuild the host (`cargo build --release -p tynd-full`) between runs, `tynd dev` picks up the new binary automatically — no re-install needed.

See also: [`playground/lite`](../lite/README.md) for the same app against the `lite` runtime.
