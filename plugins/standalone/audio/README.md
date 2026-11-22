# @tynd/plugin-audio

Audio capture (microphone) + playback (WAV files / raw samples) + WAV file
I/O for Tynd apps. Native Rust via [`cpal`][cpal] (capture/playback) and
[`hound`][hound] (WAV encoder/decoder).

**Status: implemented.** Capture (`startCapture`/`stopCapture`),
playback (`playFile`/`playSamples`/`stopPlayback`) and WAV save
(`saveWav`) all run against real `cpal` / `hound` backends — the
`audio` + `whisper` voice pipeline is operational end-to-end. Installing the
plugin through Tynd automatically enables `capture` and `wav`. A direct Cargo
dependency still defaults to a portable stub unless features are selected
explicitly (see [Feature gates](#feature-gates) below).

**Requires** a Rust toolchain on the development machine; Tynd compiles the
plugin crate into the app's host automatically.

## Install

```sh
tynd add audio
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-audio"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "Audio Demo" } })
```

## Feature gates

Tynd reads the package's `tynd.native.features` metadata and enables
`capture` + `wav` automatically. Direct Rust consumers can select them
manually:

```toml
# In your app's Cargo.toml (or via the CLI's plugin config)
[dependencies]
tynd-plugin-audio = { workspace = true, features = ["capture", "wav"] }
```

| Feature  | Pulls in           | Enables                                    |
|----------|--------------------|---------------------------------------------|
| `capture` | `cpal` + `hound`   | `listDevices`, `startCapture`, `playFile`, `playSamples`, `stopCapture`, `stopPlayback` |
| `wav`     | `hound`            | `saveWav` (32-bit float PCM)               |

`capture` implies `wav` because `playFile` decodes WAV on the fly.
Without a feature the relevant method returns `"audio.<method>: rebuild
with --features <name>"` so callers can detect the gap deterministically.

### macOS microphone permission

`startCapture` triggers macOS' microphone consent prompt the first
time it runs. Apps must declare `NSMicrophoneUsageDescription` in
their bundled `Info.plist` (or the prompt is denied silently and
`startCapture` fails with an OS-reported error). Example:

```xml
<key>NSMicrophoneUsageDescription</key>
<string>Used to capture voice for transcription.</string>
```

Linux + Windows have no permission prompt — `cpal` opens the device
directly through ALSA / WASAPI.

## Use

```ts
import { audio } from "@tynd/plugin-audio"

// 1. Discover devices
const { inputs, outputs } = await audio.listDevices()
console.log({ inputs, outputs })

// 2. Capture mic — yields Float32Array chunks until break / .stop()
const stream = audio.startCapture({ sampleRate: 44100, channels: 1 })
const chunks: Float32Array[] = []
for await (const { samples, timestamp } of stream) {
  chunks.push(samples)
  if (chunks.length >= 100) break // also calls stream.stop()
}

// 3. Save accumulated samples to a WAV file
const flat = new Float32Array(chunks.reduce((n, c) => n + c.length, 0))
let offset = 0
for (const c of chunks) { flat.set(c, offset); offset += c.length }
await audio.saveWav(flat, "/tmp/clip.wav", { sampleRate: 44100, channels: 1 })

// 4. Play it back
const handle = await audio.playFile("/tmp/clip.wav", { volume: 0.8 })
// later
await handle.stop()
```

## API

| Method | Purpose |
|---|---|
| `audio.listDevices()` | `{ inputs, outputs }` device-name arrays. |
| `audio.startCapture(opts?)` | `AsyncIterable<{ samples, timestamp }>` + `.stop()` + `.streamId`. |
| `audio.stopCapture(id)` | Stop a live capture stream. |
| `audio.playFile(path, opts?)` | Play a WAV file. Returns `{ id, durationMs, stop() }`. |
| `audio.playSamples(samples, opts)` | Play raw f32 PCM samples. |
| `audio.stopPlayback(id)` | Stop a playback session. |
| `audio.saveWav(samples, path, opts)` | Write 32-bit float WAV. |

## Why feature gates?

`cpal` carries OS-specific bindings (CoreAudio / WASAPI / ALSA) that don't
cross-compile out of the box on every CI runner. `hound` is small but
adds ~2k LoC of WAV parsing nobody uses for capture-only apps. Splitting
them out lets downstream apps pay only for what they actually use, the
same way `@tynd/plugin-llm` and `@tynd/plugin-screen-rec` keep their
heavy native deps behind Cargo features while package metadata enables the
advertised Tynd baseline automatically.

## Voice pipeline (audio + whisper)

The end-to-end live transcription pipeline now works:

```ts
import { audio }   from "@tynd/plugin-audio"
import { whisper } from "@tynd/plugin-whisper"

const model = await whisper.load({ path: "./ggml-tiny.bin" })
const stream = audio.startCapture({ sampleRate: 16_000, channels: 1 })

async function* asSamples(s: AsyncIterable<{ samples: Float32Array }>) {
  for await (const c of s) yield c.samples
}

for await (const seg of whisper.transcribeStream(model.id, asSamples(stream))) {
  console.log(seg.text)
}
```

## Capture events

Capture streams emit three event kinds (filtered by `id` on the client):

| Event           | When                                                   |
|-----------------|--------------------------------------------------------|
| `audio:samples` | Per-callback chunk: `{ id, samples (b64), timestamp }` |
| `audio:warn`    | `sample-rate-fallback` / `channel-fallback` |
| `audio:error`   | OS reported a stream error (device disconnect, xrun)   |

All OS/plugin events pass through the host's bounded 1,024-event queue and are
delivered to webviews in batches of 64. If a renderer remains stalled at
capacity, new events are dropped and the host emits one aggregated diagnostic
instead of allowing unbounded memory growth.

## Concurrency bounds (`playFile` / `playSamples`)

The plugin gates playback through two independent pools so a runaway
caller can't OOM the host:

| Pool             | Size                              | Held until                        |
|------------------|-----------------------------------|-----------------------------------|
| Decode permits   | `available_parallelism()`         | Decode + stream-build returns (~ms). Caps concurrent `Vec<f32>` allocations during WAV decode (each up to 256 MiB). |
| Playback slots   | `4 × available_parallelism()`     | `stopPlayback` / EOF — released when the cpal stream is dropped from the registry. Caps concurrent live streams. |

A `playFile` / `playSamples` call takes one of each. If the slot pool
is exhausted, the call blocks until a previous playback finishes. The
slot is sized larger than the decode pool because live playback is
mostly idle DMA — a host can hold more concurrent streams than active
decodes.

[cpal]: https://github.com/RustAudio/cpal
[hound]: https://github.com/ruuda/hound

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
