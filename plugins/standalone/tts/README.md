# @tynd/plugin-tts

Text-to-speech for Tynd apps. Native Rust via the cross-platform
[`tts`][tts] crate, which wraps:

| Platform | Engine                              |
|----------|-------------------------------------|
| Windows  | SAPI 5                              |
| macOS    | `AVSpeechSynthesizer`               |
| iOS      | `AVSpeechSynthesizer`               |
| Linux    | `speech-dispatcher` / espeak        |

**Status: implemented.** Installing the plugin through Tynd automatically
enables the native `synthesis` backend. Direct Cargo consumers can opt in
manually:

```toml
[dependencies]
tynd-plugin-tts = { workspace = true, features = ["synthesis"] }
```

Without it, the feature-less direct Cargo build is a deterministic stub.

**Requires** a Rust toolchain on the development machine; Tynd compiles the
plugin crate into the app's host automatically.

## Install

```sh
tynd add tts
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-tts"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "Reader" } })
```

## Use

```ts
import { tts } from "@tynd/plugin-tts"

const voices = await tts.listVoices()
console.log(voices)

await tts.speak("Hello from Tynd!", {
  voice: voices.find((v) => v.language.startsWith("en"))?.id,
  rate: 1.0,   // 0..2, 1 = normal speed
  pitch: 1.0,  // 0..2, 1 = normal pitch
  volume: 0.8, // 0..1
})

while (await tts.isSpeaking()) {
  await new Promise((r) => setTimeout(r, 100))
}

await tts.stop() // idempotent — safe to call when idle
```

## API

| Method | Purpose |
|---|---|
| `tts.listVoices()` | Installed voices on the host. |
| `tts.speak(text, opts?)` | Queue an utterance behind any speech in progress (Windows, macOS and Linux backends all queue). Resolves on hand-off (not completion). To interrupt, call `tts.stop()` first. |
| `tts.stop()` | Cancel in-progress speech and clear the queue. |
| `tts.isSpeaking()` | Polling-style "is the synth busy right now?". |

## Units

`rate`, `pitch` and `volume` use the same scale on every OS and are mapped
onto each backend's native range (speech-dispatcher `-100..100`,
AVFoundation `0.1..2` rate, WinRT `0.5..6` rate, ...). `rate` / `pitch`:
`0..2` where `1` is the backend's normal value; `volume`: `0..1`. Values
outside the range are clamped. Omitted knobs reset to the backend default.

## Limitations

### `synthesize()` (text → WAV file) — not exposed

The cross-platform [`tts`][tts] crate doesn't surface a "render to
file" path uniformly across SAPI / AVSpeechSynthesizer / espeak — each
engine *can* render to a buffer (SAPI's `SpFileStream`, AVSpeech's
`write(_:bufferCallback:)`, espeak's `--stdout`), but the abstraction
stops at the play-to-speaker path. To avoid shipping a method that
silently rejects every call, `synthesize()` is **not** part of the
public surface.

**Workaround**: drive [`@tynd/plugin-audio`][audio]'s `startCapture` in
parallel with `tts.speak`:

```ts
import { audio } from "@tynd/plugin-audio"
import { tts } from "@tynd/plugin-tts"

const handle = await audio.startCapture({
  sampleRate: 16000,
  channels: 1,
  outputPath: "/tmp/utterance.wav",
})
await tts.speak("Hello from Tynd!")
while (await tts.isSpeaking()) {
  await new Promise((r) => setTimeout(r, 50))
}
await audio.stopCapture(handle.id)
```

A future revision may add `tts.synthesize()` back behind per-OS
backends (SAPI's `SpFileStream`, AVSpeech's `write(_:bufferCallback:)`,
`espeak --stdout`) once the wire-format question is resolved.

[audio]: ../audio

## Why feature gates?

The `tts` crate pulls in:

- **Linux**: bindings against `libspeechd` / `libespeak-ng`.
- **macOS / iOS**: AppKit / `AVFoundation` glue.
- **Windows**: SAPI COM bindings.

That dependency tree adds non-trivial build friction. The feature gate keeps
direct Cargo tooling lightweight; Tynd package metadata enables it
automatically only for apps that install this plugin.

[tts]: https://crates.io/crates/tts

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
