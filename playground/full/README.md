# playground/full — Tynd Chat

A working local-LLM chatbot built on Tynd's **`full`** runtime. Connects to any OpenAI-compatible server running on `localhost:<port>` (editable from the header) and streams token-by-token replies into a React UI via Tynd's streaming RPC.

```bash
# From the repo root (one-time)
cargo build --release -p tynd-full
bun install

# Start any OpenAI-compatible server on a port of your choice, e.g.:
#   Ollama      (default 11434)
#   LM Studio   (default 1234)
#   llama.cpp   (default 8080)
#   vLLM        (default 8000)

# Then from this directory
bun run dev          # tynd dev — HMR frontend + hot-reload backend
bun run build        # tynd build — single packaged binary in release/
bun run start        # tynd start — run cached bundles, no rebuild
```

Enter the port in the header next to the status dot — it's persisted across launches via `localStorage`. The dot pings `/api/v1/models` whenever the URL changes:

- 🟢 server reachable
- 🟡 probing
- 🔴 unreachable — start your server or fix the port

Default port: `13305`.

## What this demonstrates

- **Streaming RPC end-to-end.** Backend is an `async function*` that parses SSE from `fetch` and `yield`s token deltas. Frontend iterates with `for await` — batches arrive every ~10 ms (streaming batching), credits replenish every 32 tokens (backpressure), cancel on Stop tears down the HTTP request cleanly.
- **Zero-glue typed RPC.** `createBackend<typeof backend>()` on the frontend — no codegen, no IDL. `ChatMessage` / `ChatOptions` types flow from backend straight into the UI.
- **Bun-native fetch + SSE parsing.** The backend pipes `ReadableStream` + `TextDecoder` + JSON parsing without polyfills — the kind of code that justifies the `full` runtime.
- **Cancelable generation.** The Stop button calls `handle.cancel()`, which propagates through Tynd's cancel protocol to the `finally` block of the async generator — which aborts the `fetch`, which closes the upstream socket. No dangling requests on the server.

## Layout

```
playground/full/
├── backend/
│   └── main.ts                    ← chat() generator + pingServer()
├── src/
│   ├── api.ts                     ← shared backend proxy + URL helper
│   ├── types.ts                   ← UiMessage + ServerStatus
│   ├── hooks/
│   │   ├── useChat.ts             ← conversation state + streaming lifecycle
│   │   ├── useServerStatus.ts     ← reactive health probe
│   │   └── usePersistedPort.ts    ← port state + localStorage persistence
│   ├── components/
│   │   ├── ChatHeader.tsx
│   │   ├── ChatInput.tsx
│   │   ├── MessageList.tsx
│   │   ├── Message.tsx
│   │   ├── PortInput.tsx
│   │   └── StatusDot.tsx
│   ├── App.tsx                    ← thin orchestrator
│   ├── App.css
│   └── main.tsx
├── tynd.config.ts                 ← runtime: "full"
└── vite.config.ts
```

The default base URL is `http://localhost:13305` and the expected endpoints are `POST /api/v1/chat/completions` (SSE) plus `GET /api/v1/models` (health). If your server uses a different path prefix, edit `chatUrl()` / `modelsUrl()` in `backend/main.ts`.

See also: [`playground/example`](../example/README.md) — same framework, minimal demo, `lite` runtime.
