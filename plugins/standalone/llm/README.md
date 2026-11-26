# @tynd/plugin-llm

Local LLM inference for Tynd apps. Loads GGUF checkpoints (Llama,
Mistral, Phi, Qwen, …) through [`llama-cpp-2`][lc2] (Rust binding for
[`llama.cpp`][lcpp]) and streams tokens back to the frontend.

**Requires** a Rust toolchain on the development machine; Tynd compiles the
plugin crate into the app's host automatically.

> **Experimental**: CPU inference is enabled automatically when installed
> through Tynd, but the required C/C++/CMake/libclang build has not yet passed
> the full supported desktop matrix. npm publishes this package under the
> `experimental` dist-tag. See [Limitations](#limitations).

## Install

```sh
tynd add llm
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-llm"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "My LLM App" } })
```

The package metadata enables `inference` automatically. For GPU acceleration,
add one target-specific feature; it implies `inference`:

```ts
plugins: [
  { pkg: "@tynd/plugin-llm", features: ["metal"] }, // or cuda / vulkan
]
```

Only direct Cargo consumers see the feature-less stub unless they enable
`inference` themselves.

## Quickstart

```ts
import { llm } from "@tynd/plugin-llm"

// One-time: download a GGUF model from Hugging Face — see "Models" below.
const model = await llm.load({
  path: "/home/me/models/mistral-7b-instruct-v0.2.Q4_K_M.gguf",
  contextSize: 4096,
  gpuLayers: 32, // 0 for CPU-only
})

// Streaming
let out = ""
for await (const piece of llm.generate(model.id, "Write a haiku about Rust:", {
  maxTokens: 64,
  temperature: 0.8,
})) {
  out += piece
  console.log(out) // grows token-by-token
}

// One-shot
const answer = await llm.complete(model.id, "What is 2 + 2?", { maxTokens: 16 })

await llm.unload(model.id)
```

## Use with Vercel AI SDK

For chat UX with conversation state, history, tool calling, multimodal,
and React/Vue/Svelte/Solid hooks, use [Vercel AI SDK][vercel-ai] on top
of this plugin's local llama.cpp engine.

### Install peer deps

```sh
bun add ai @ai-sdk/provider
```

### Wire it up

```ts
import { llm } from "@tynd/plugin-llm"
import { tyndLlmProvider } from "@tynd/plugin-llm/ai-sdk"
import { streamText } from "ai"

const model = await llm.load({ path: "/home/me/models/mistral-7b.gguf" })

const result = streamText({
  model: tyndLlmProvider(model.id, { name: "mistral-7b" }),
  messages: [
    { role: "system", content: "You are a helpful assistant." },
    { role: "user", content: "Explain quantum entanglement in simple terms." },
  ],
})

for await (const chunk of result.textStream) {
  process.stdout.write(chunk)
}
```

### What you get

By bridging into the AI SDK you get **for free**:

- Conversation state + message history.
- React `useChat()` / Vue / Svelte / Solid UI hooks.
- Stream events (`text-delta`, `finish`, `error`, `stream-start` …).
- AbortSignal support — wired through to the local generator.
- Standard `warnings` array surfacing unsupported settings.
- Type-safe tool calling glue (when the underlying model can produce it).

### Limitations of the current adapter

- **Tool calling**: not supported. The llama.cpp engine doesn't natively
  format / parse tool calls — apps that need tools should use a model
  fine-tuned for tool use and parse the JSON responses themselves. The
  adapter emits an `unsupported` warning when `tools` is set.
- **Structured JSON (`responseFormat: "json"`)**: not constrained at the
  decoder level. Prompt the model for JSON instead. The adapter emits a
  warning when set.
- **Multimodal images / PDFs**: not supported — text-only models. File
  parts in user messages are silently dropped.
- **Token usage**: not tracked yet (the llm plugin doesn't expose
  prompt / completion token counts; will land at the engine layer).
- **Prompt formatting**: uses a simple `role: content\n` flatten,
  terminated with `assistant:`. Models with chat templates baked into
  the GGUF (Mistral, Llama-3 ChatML, Qwen, …) usually still produce
  coherent output. For stricter control, pre-format the prompt and call
  `llm.generate` directly.

### When NOT to use this adapter

- **Cloud LLMs (OpenAI, Anthropic, Google, Mistral cloud)** — install
  the matching provider package directly (`@ai-sdk/openai`,
  `@ai-sdk/anthropic`, …); no need for this plugin at all.
- **Raw token streaming with no chat ceremony** — use `llm.generate()`
  directly. Lighter, simpler, no AI SDK runtime in the bundle.

[vercel-ai]: https://sdk.vercel.ai

## API

| Method | Purpose |
|---|---|
| `llm.load({ path, contextSize?, gpuLayers? })` | Open a GGUF file; returns `LlmModel` handle. |
| `llm.generate(id, prompt, opts?)` | Stream `AsyncIterable<string>` of tokens. |
| `llm.complete(id, prompt, opts?)` | One-shot — returns the full string. |
| `llm.cancel(streamId)` | Cancel an in-flight `generate(...)` stream. Idempotent. |
| `llm.unload(id)` | Free the model's memory. Idempotent. |
| `llm.list()` | Snapshot of currently-loaded models. |

`GenerateOptions`:

| Field | Default | Range | Notes |
|---|---|---|---|
| `maxTokens` | `512` | 1+ | Hard cap on generation length. |
| `temperature` | `0.7` | 0–2 | 0 ≈ greedy, > 1 = creative. |
| `topK` | `40` | 0+ | 0 disables. |
| `topP` | `0.9` | 0–1 | Nucleus sampling threshold. |
| `seed` | random | u64 | Pin for deterministic output. |
| `stop` | `[]` | strings | Halt as soon as one matches the cumulative output. The matched text (and anything after it) is never streamed; text that might be the start of a stop sequence is held back until it is ruled out. |
| `signal` (client only) | — | `AbortSignal` | Stops iteration locally — see [Abort semantics](#abort-semantics). |

## Models

GGUF is the on-disk format `llama.cpp` consumes. Pick a model that
matches the target hardware's RAM (full-precision Q8 → ~2× param
count in GB; quantised Q4 → ~0.6×).

Common starting points (Apache-2.0 / MIT-licensed quants on Hugging
Face):

| Model | Size (Q4_K_M) | Use case |
|---|---|---|
| [`TheBloke/Mistral-7B-Instruct-v0.2-GGUF`][mistral] | ~4.4 GB | General chat, low latency. |
| [`microsoft/Phi-3-mini-4k-instruct-gguf`][phi] | ~2.3 GB | Lightweight, runs on 8 GB RAM. |
| [`Qwen/Qwen2.5-7B-Instruct-GGUF`][qwen] | ~4.7 GB | Multilingual + code. |
| [`bartowski/Meta-Llama-3.1-8B-Instruct-GGUF`][llama3] | ~4.9 GB | Strong English baseline. |

Download with `huggingface-cli` or `wget`:

```sh
huggingface-cli download TheBloke/Mistral-7B-Instruct-v0.2-GGUF \
  mistral-7b-instruct-v0.2.Q4_K_M.gguf --local-dir ~/models
```

## Performance

| Backend | Throughput (Mistral-7B Q4_K_M, prompt+gen) | Notes |
|---|---|---|
| CPU only (`gpuLayers: 0`) | 5–15 tok/s | Modern x86 / Apple Silicon. |
| Partial GPU offload (`gpuLayers: 20`) | 30–60 tok/s | Mid-range NVIDIA / AMD. |
| Full offload (`gpuLayers: 999`) | 80–200 tok/s | High-VRAM dGPU (≥ 12 GB). |

Numbers are rough — see [llama.cpp's perplexity / speed
benchmarks][bench] for hardware-specific data. Set `gpuLayers` larger
than the model's actual layer count to offload everything; llama.cpp
caps it internally.

## Memory

Rule of thumb for `Q4_K_M` quantisation:

- 7B param ≈ 5 GB resident
- 13B param ≈ 9 GB resident
- 70B param ≈ 42 GB resident (rare on consumer hardware)

Add `contextSize × n_kv_layers × 2 × bytes_per_token` for the KV
cache (typically 0.5–4 GB depending on model + ctx size). The plugin
caps at 16 concurrently loaded models — bump `MAX_MODELS` in
`native/src/lib.rs` if a use case demands more.

Concurrent `generate()` / `complete()` calls are capped at
`std::thread::available_parallelism()` permits. A call waits up to 15 s
for a free permit and then fails with a "busy" error (it never blocks the
host's call pool indefinitely), spawns one inference thread, and the permit
returns to the pool on completion or cancel. A bursty caller cannot spawn N
llama.cpp threads each saturating the GPU/CPU. A stream whose `startStream`
handshake does not arrive within 30 s (for example after a page reload)
expires: its permit is released and `llm:end { ok: false }` is emitted.
Inference runs without holding the model registry lock, so `list()`,
`unload()` and other runs on the same model are never blocked by a long
generation. At app exit every stream is cancelled and every model released.

## Abort semantics

`generate` accepts an `AbortSignal` on the client side. When aborted,
the `AsyncIterable` stops yielding **immediately**. To also stop the
underlying native generation thread, call `llm.cancel(streamId)` —
the host inference loop checks the cancel flag on every token and
breaks. The terminating `llm:end` event still fires.

`complete` without a signal uses the synchronous native `complete`
arm; with a signal it falls back to draining `generate` so the abort
takes effect locally.

## Limitations

- **Toolchain**: native builds require a C/C++ compiler, CMake and libclang.
  Hardware acceleration (`cuda`, `metal`, `vulkan`) remains app-selected.
- **Build time**: first-time compile of `llama.cpp` is 5–15 minutes
  on a cold machine; incremental rebuilds are fast.
- **Single backend init**: `LlamaBackend::init()` is process-global
  in upstream llama.cpp. The first `load()` initialises it; subsequent
  loads share the same `Arc`.
- **Licensing**: `llama.cpp` is MIT-licensed; bundled GGUF weights
  carry their own licenses (Llama-3 → Meta Community License, Mistral
  → Apache-2.0, Phi-3 → MIT). Audit each model before shipping.

## Why not core?

LLM inference is a heavy specialised workload, not an OS primitive.
Keeping it out of `@tynd/core` lets apps that don't install it pay zero build
or binary cost. Installed apps deliberately accept the C++ toolchain and
llama.cpp footprint.

[lc2]: https://github.com/utilityai/llama-cpp-rs
[lcpp]: https://github.com/ggerganov/llama.cpp
[mistral]: https://huggingface.co/TheBloke/Mistral-7B-Instruct-v0.2-GGUF
[phi]: https://huggingface.co/microsoft/Phi-3-mini-4k-instruct-gguf
[qwen]: https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF
[llama3]: https://huggingface.co/bartowski/Meta-Llama-3.1-8B-Instruct-GGUF
[bench]: https://github.com/ggerganov/llama.cpp/blob/master/examples/perplexity/README.md

---

See also: [Standalone plugin index](../README.md).
