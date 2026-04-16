# react-lite

Vorn playground app — React + Vite frontend, **lite** runtime (QuickJS embedded, no external runtime on target).

## Stack

- Frontend: React + TypeScript + Vite
- Backend: TypeScript (QuickJS embedded via `vorn-lite`)
- Runtime: `lite` — single binary, ~5 MB smaller than full, ~20 ms startup

## Development

```bash
bun run dev        # starts Vite dev server + vorn host
```

## Build

```bash
bun run build      # → release/react-lite[.exe]  (~2-3 MB, no runtime needed on target)
```

The built binary is self-contained: `vorn-lite` binary + QuickJS bundle + frontend assets packed as `VORNPKG`.

## Project layout

```
react-lite/
├── vorn.config.ts        ← runtime: "lite"
├── backend/
│   └── main.ts           ← app.start() + exported functions + events
├── src/                  ← React frontend
│   ├── App.tsx
│   └── main.tsx
└── public/
    └── favicon.png       ← auto-used as window icon + .exe icon
```
