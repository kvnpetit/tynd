# react-full

Vorn playground app — React + Vite frontend, **full** runtime (Bun subprocess, full npm ecosystem).

## Stack

- Frontend: React + TypeScript + Vite
- Backend: TypeScript (Bun subprocess via `vorn-full`)
- Runtime: `full` — Bun runtime embedded in the binary, complete npm/Node.js API access

## Development

```bash
bun run dev        # starts Vite dev server + vorn host
```

## Build

```bash
bun run build      # → release/react-full[.exe]  (~60-80 MB, runtime included)
```

The built binary is self-contained: compiled launcher + `vorn-full` host + backend bundle + frontend assets.

## Project layout

```
react-full/
├── vorn.config.ts        ← runtime: "full"
├── backend/
│   └── main.ts           ← app.start() + exported functions + events
├── src/                  ← React frontend
│   ├── App.tsx
│   └── main.tsx
└── public/
    └── favicon.png       ← auto-used as window icon + .exe icon
```
