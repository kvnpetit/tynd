# @tynd/plugin-whisper

Local speech-to-text for Tynd apps. Loads OpenAI Whisper checkpoints
(GGML / GGUF — tiny / base / small / medium / large) through
[`whisper-rs`][wrs] (Rust binding for [`whisper.cpp`][wcpp]) and streams
transcribed segments back to the frontend.

**Requires** a Rust toolchain on the development machine; Tynd compiles the
plugin crate into the app's host automatically.

> **Experimental**: CPU transcription is enabled automatically when installed
> through Tynd, but the required C/C++/CMake/libclang build has not yet passed
> the full supported desktop matrix. npm publishes this package under the
> `experimental` dist-tag. See [Limitations](#limitations).

## Install

```sh
tynd add whisper
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-whisper"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "Voice Notes" } })
```

The package metadata enables `inference` automatically. For acceleration, add
one target-specific feature; it implies `inference`:

```ts
plugins: [
  { pkg: "@tynd/plugin-whisper", features: ["metal"] },
]
```

Other choices are `cuda`, `vulkan`, `hipblas`, and `openblas`. Only direct
Cargo consumers see the feature-less stub.

## Quickstart

```ts
import { whisper } from "@tynd/plugin-whisper"

// One-time: download a GGML model from Hugging Face — see "Models" below.
const model = await whisper.load({
  path: "/home/me/models/ggml-base.bin",
  gpu: true, // false to force CPU
})

// Whisper expects 16 kHz mono f32 PCM — load a clip via your favourite
// audio plugin / Web Audio decoder.
const samples = await load16kMono("/home/me/clip.wav")

// Streaming — segments are delivered once whisper.cpp has decoded the buffer.
for await (const seg of whisper.transcribe(model.id, samples, {
  language: "auto",      // or "en" / "fr" / "ja" / …
  wordTimestamps: true,
})) {
  console.log(`[${seg.startMs}-${seg.endMs}]`, seg.text)
  for (const w of seg.words ?? []) {
    console.log("  ", w.text, w.confidence.toFixed(2))
  }
}

// One-shot
const transcript = await whisper.transcribeAll(model.id, samples, { language: "en" })

// Pre-pass language detection
const { language, confidence } = await whisper.detectLanguage(model.id, samples)

await whisper.unload(model.id)
```

## Full voice pipeline

Whisper composes with `@tynd/plugin-audio` (capture),
`@tynd/plugin-llm` (process), and `@tynd/plugin-tts` (response) into a
fully-local voice assistant. Nothing leaves the device.

```ts
import { audio } from "@tynd/plugin-audio"
import { llm } from "@tynd/plugin-llm"
import { tts } from "@tynd/plugin-tts"
import { whisper } from "@tynd/plugin-whisper"

// 1. Load the local models once at boot.
const stt = await whisper.load({ path: "/models/ggml-base.bin" })
const reasoning = await llm.load({ path: "/models/mistral-7b.gguf", gpuLayers: 32 })

// 2. Capture mic at whisper's required sample rate.
const stream = audio.startCapture({ sampleRate: 16_000, channels: 1 })

// 3. Strip the `{ samples, timestamp }` envelope so the iterator yields
//    raw Float32Array chunks for `transcribeStream`.
async function* asSamples(s: AsyncIterable<{ samples: Float32Array }>) {
  for await (const c of s) yield c.samples
}

// 4. Live transcribe in 30 s windows.
let userQuestion = ""
for await (const seg of whisper.transcribeStream(stt.id, asSamples(stream), {
  language: "en",
  windowMs: 30_000,
})) {
  userQuestion += seg.text
  if (userQuestion.endsWith("?")) break // simple end-of-utterance cue
}
await stream.stop()

// 5. Hand the transcript to the LLM.
let answer = ""
for await (const tok of llm.generate(reasoning.id, userQuestion, { maxTokens: 256 })) {
  answer += tok
}

// 6. Speak the response.
await tts.speak(answer)

// Cleanup (or keep models warm for the next turn).
await whisper.unload(stt.id)
await llm.unload(reasoning.id)
```

## API

| Method | Purpose |
|---|---|
| `whisper.load({ path, gpu? })` | Open a GGML / GGUF model; returns `WhisperModel` handle. |
| `whisper.transcribe(id, samples, opts?)` | Stream `AsyncIterable<TranscriptSegment>`. |
| `whisper.transcribeAll(id, samples, opts?)` | One-shot — concatenated string. |
| `whisper.transcribeStream(id, audioStream, opts?)` | Live transcription from an `AsyncIterable<Float32Array>` (e.g. `@tynd/plugin-audio` capture). |
| `whisper.detectLanguage(id, samples)` | `{ language, confidence }` from whisper's language head. |
| `whisper.cancel(streamId)` | Cancel an in-flight `transcribe(...)`; the native decode aborts promptly. Idempotent. |
| `whisper.unload(id)` | Free the model's memory. Idempotent. |
| `whisper.list()` | Snapshot of currently-loaded models. |

`TranscribeOptions`:

| Field | Default | Notes |
|---|---|---|
| `language` | `"auto"` | BCP-47 short code, or `"auto"` for detection. |
| `translate` | `false` | Translate recognised speech to English. |
| `threads` | `0` | `0` = `available_parallelism()`. CPU only. |
| `prompt` | — | Initial prompt to bias vocabulary / formatting. |
| `temperature` | `0.0` | `0.0` = greedy; raise for more diverse decoding. |
| `maxLen` | — | Max segment length in characters. |
| `wordTimestamps` | `false` | Emit per-word timings + confidence. |
| `signal` (client only) | — | `AbortSignal` to stop iteration locally. |

## Models

Whisper ships in five sizes; download from
[`ggerganov/whisper.cpp`][hf] on Hugging Face:

| Model | Size (Q5_0) | Disk | Speed (CPU) | Notes |
|---|---|---|---|---|
| `ggml-tiny.bin` | 75 MB | ~75 MB | ~32× realtime | Live captions, low quality. |
| `ggml-base.bin` | 142 MB | ~142 MB | ~16× realtime | Good quickstart default. |
| `ggml-small.bin` | 466 MB | ~466 MB | ~6× realtime | Strong general use. |
| `ggml-medium.bin` | 1.5 GB | ~1.5 GB | ~2× realtime | Near-large quality. |
| `ggml-large-v3.bin` | 3.0 GB | ~3.0 GB | ~1× realtime | Best quality. |

English-only variants (`*.en.bin`) are smaller and faster but won't
auto-detect language. Quantised variants (`-q5_0`, `-q8_0`) trade a
little quality for ~30-50% smaller weights.

Download with `wget` or `huggingface-cli`:

```sh
wget https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin \
  -O ~/models/ggml-base.bin

# Or for the bigger checkpoints:
huggingface-cli download ggerganov/whisper.cpp ggml-medium.bin \
  --local-dir ~/models
```

## GPU acceleration

Pick the feature flag that matches the target hardware:

| Feature   | Target                                        |
|-----------|-----------------------------------------------|
| `metal`   | Apple Silicon (M1+) — best Mac throughput.    |
| `cuda`    | NVIDIA GPUs (CUDA toolkit must be installed). |
| `vulkan`  | Cross-vendor (NVIDIA / AMD / Intel) — Vulkan SDK required. |
| `hipblas` | AMD ROCm (Linux mostly).                      |
| `openblas`| CPU-side BLAS speedup (no GPU).               |

```ts
// tynd.config.ts — pick exactly one accelerator
plugins: [
  { pkg: "@tynd/plugin-whisper", features: ["metal"] },
]
```

The `gpu: true` option on `load()` is honoured only when the crate was
built with one of these features; otherwise the call falls back to CPU.

## Performance

Indicative numbers on a Whisper `base` model (CPU = M1 Pro 8-core, GPU
= RTX 3070 mobile). Real-world throughput swings by audio length and
model size.

| Model | CPU | Metal (Apple Silicon) | CUDA (RTX 3070) |
|---|---|---|---|
| tiny  | ~32× RT | ~80× RT  | ~110× RT |
| base  | ~16× RT | ~50× RT  | ~75× RT  |
| small | ~6× RT  | ~20× RT  | ~35× RT  |
| medium | ~2× RT | ~8× RT   | ~15× RT  |
| large | ~1× RT  | ~3× RT   | ~6× RT   |

(`× RT` = realtime multiplier — `10× RT` means 10 s of audio
transcribed per second of wall time.)

See [whisper.cpp's benchmarks][bench] for hardware-specific data.

## Memory

Rule of thumb when loaded:

- `tiny`: ~150 MB
- `base`: ~300 MB
- `small`: ~900 MB
- `medium`: ~2.5 GB
- `large-v3`: ~5 GB

The plugin caps at 8 concurrently loaded models — bump `MAX_MODELS`
in `native/src/lib.rs` if a use case demands more.

Concurrent `transcribe()` calls are capped at
`std::thread::available_parallelism()` permits. A call waits up to 15 s
for a free permit and then fails with a "busy" error (it never blocks the
host's call pool indefinitely), spawns one decoder thread, and the permit
returns to the pool on completion or cancel. A stream whose `startStream`
handshake does not arrive within 30 s (for example after a page reload)
expires: its permit is released and `whisper:end { ok: false }` is emitted.
Inference runs without holding the model registry lock, so `list()`,
`unload()` and other runs on the same model are never blocked by a long
transcription. `threads` is clamped to `available_parallelism()`. At app
exit every stream is cancelled and every model released.

## Abort semantics

`transcribe` accepts an `AbortSignal` on the client side. When
aborted, the `AsyncIterable` stops yielding **immediately**. To also
stop the underlying native decode, call `whisper.cancel(streamId)` —
the host sets a flag that whisper.cpp polls while decoding, so the
decode stops within a fraction of a second. Segments are emitted only
after the whole buffer is decoded, so a cancelled run emits none. The terminating `whisper:end` event still
fires. `transcribeAll` without a signal uses the synchronous native
arm; with a signal it falls back to draining `transcribe` so the
abort takes effect locally.

## Limitations

- **Toolchain**: native builds require a C/C++ compiler, CMake and libclang.
  Hardware acceleration remains app-selected.
- **Build time**: first-time compile of whisper.cpp is 3-5 minutes on
  a cold machine; incremental rebuilds are fast.
- **Sample rate**: whisper hard-requires **16 kHz mono f32 PCM**.
  Resample upstream (Web Audio `AudioBuffer.copyFromChannel` after a
  `AudioContext({ sampleRate: 16000 })` or `cpal` device pinned to
  16 kHz). Wrong rate ≈ unintelligible output. The backend now
  rejects calls whose `sampleRate` / `channels` args don't match
  (`16000` / `1`) so the misconfiguration fails loudly instead of
  emitting silent gibberish — pass the actual capture-side values via
  the `sampleRate` and `channels` keys on `transcribe` /
  `transcribeAll` / `detectLanguage` (both default to the required
  values when omitted, so existing callers that already produce
  correctly-formatted PCM are unaffected).
- **30 s window cap**: whisper's encoder only sees 30 s at a time. The
  `transcribeStream` helper buffers chunks into 30 s windows — for
  arbitrary-length live audio, layer a Voice Activity Detector
  upstream so windows align with utterance boundaries.
- **Language detection confidence**: `detectLanguage` reports the
  probability of the winning language (0..1) from whisper's language head.
- **No tool calling / structured output**: whisper is pure
  transcription. For meaning extraction (entities, intents, summaries),
  pipe the text into `@tynd/plugin-llm`.

## Why not core?

Speech-to-text is a heavy specialised workload, not an OS primitive.
Keeping it out of `@tynd/core` lets apps that don't install it pay zero build
or binary cost. Installed apps deliberately accept the C++ toolchain and
whisper.cpp footprint.

[wrs]: https://github.com/tazz4843/whisper-rs
[wcpp]: https://github.com/ggerganov/whisper.cpp
[hf]: https://huggingface.co/ggerganov/whisper.cpp/tree/main
[bench]: https://github.com/ggerganov/whisper.cpp#memory-usage

---

See also: [Standalone plugin index](../README.md).
