# @tynd/plugin-screen-rec

Cross-platform screen recording for Tynd apps. Native Rust via
[`scap`][scap] — ScreenCaptureKit (macOS 12.3+), DXGI Desktop
Duplication (Windows 10+), PipeWire portal / XCB (Linux).

**Requires** a Rust toolchain; Tynd compiles the plugin crate into the app's
host automatically and enables the baseline `capture` feature declared by the package.

> **Experimental**: baseline capture and source selection compile and are
> tested locally, but the optional codec/platform matrix is incomplete. npm
> publishes this package under the `experimental` dist-tag.

## Install

```sh
tynd add screen-rec
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-screen-rec"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "Recorder" } })
```

## Use

```ts
import { screenRec } from "@tynd/plugin-screen-rec"

// Permission probe + prompt (macOS only triggers a system dialog).
if (!(await screenRec.hasPermission())) {
  await screenRec.requestPermission()
}

// Start a recording.
const rec = await screenRec.start({
  fps: 30,
  output: "/tmp/demo.mjpg",
})

// ...later
const report = await screenRec.stop({ recordingId: rec.id })
console.log(`recorded ${report.durationMs} ms (${report.sizeBytes} bytes)`)

// Pause / resume mid-stream:
await screenRec.pause({ recordingId: rec.id })
await screenRec.resume({ recordingId: rec.id })
```

## API

| Method | Purpose |
|---|---|
| `screenRec.listSources()` | List captureable displays and windows. Linux portals may return empty arrays because selection is interactive. |
| `screenRec.hasPermission()` | Check the OS screen-capture grant (macOS only). |
| `screenRec.requestPermission()` | Trigger the OS prompt (macOS only). |
| `screenRec.start(opts)` | Open a recording. Returns a `RecordingHandle`. |
| `screenRec.stop({ recordingId })` | Close a recording. Returns `{ durationMs, sizeBytes }`. |
| `screenRec.pause({ recordingId })` / `resume({ recordingId })` | Pause/resume. Idempotent. |
| `screenRec.list()` | Snapshot of currently active recording handles. |

`RecordingOptions` fields:

| Field | Default | Notes |
|---|---|---|
| `fps` | `30` | Target frame rate. |
| `region` | full screen | `{ x, y, width, height }` in **physical** pixels. |
| `monitor` | none | Display ID returned by `listSources().monitors`. Omit on Linux when the capture portal selects the source interactively. |
| `audio` | `false` | Reserved; `true` is rejected because audio muxing is not implemented. |
| `format` | none | Reserved; non-empty overrides are rejected. The codec/output extension selects the container. |
| `output` | required | Destination path (created/truncated on `start`). |
| `codec` | `"mjpeg"` | `"mjpeg" \| "h264" \| "hevc"`. The latter two need the `gpu-encode` Cargo feature. |
| `encoder` | `"auto"` | Hardware backend choice for H.264 / HEVC — see below. Ignored for MJPEG. |
| `bitrate` | `8000` | Target bitrate in **kbps**. Ignored for MJPEG (which is quality-driven). |
| `preset` | `"fast"` | `"ultrafast" \| "fast" \| "medium" \| "slow"`. Ignored for MJPEG. |

## Build flavours

| Cargo feature | Effect |
|---|---|
| _none_ (default) | Stub backend — `start()` rejects with a clear error, `hasPermission()` returns `false`. The plugin still registers + dispatches, so the host bundle layout doesn't change between dev and release. |
| `capture` | Pulls `scap` 0.0.8 + its native deps (~10 MB). Enables real screen capture and the **MJPEG** encoder. |
| `gpu-encode` | Pulls `ffmpeg-next` (links the system ffmpeg libs — see "System requirements" below). Enables hardware-accelerated **H.264** / **HEVC** encoding via NVENC, VideoToolbox, QuickSync, VA-API, with `libx264` / `libx265` CPU fallback. Implies `capture`. |

`capture` is package-required and automatic. Add `gpu-encode` to the plugin
entry only when H.264/HEVC is needed. Switching flavours triggers a full Rust
rebuild.

## Hardware-accelerated encoding (H.264 / HEVC)

The default `mjpeg` codec produces files ~10× larger than H.264 and
maxes out the CPU on a 1080p capture. Behind the `gpu-encode` feature,
the plugin grows a real H.264 / HEVC pipeline backed by `ffmpeg-next`
that drives whatever hardware encoder ships with the host's GPU.

### System requirements

`ffmpeg-next` links against the system ffmpeg shared libraries — they
must be installed (with development headers) at build time:

| Platform | Install |
|---|---|
| **macOS** | `brew install ffmpeg` |
| **Linux** | `apt install libavcodec-dev libavformat-dev libavfilter-dev libavdevice-dev libswscale-dev pkg-config` |
| **Windows** | `vcpkg install ffmpeg`, **or** download the [shared ffmpeg build](https://www.gyan.dev/ffmpeg/builds/) and set `FFMPEG_DIR=<extracted dir>` before `cargo build`. |

### Build

```sh
cd plugins/standalone/screen-rec/native
cargo build --features gpu-encode
```

Or in your app's `tynd.config.ts`:

```ts
plugins: [
  { pkg: "@tynd/plugin-screen-rec", features: ["gpu-encode"] },
]
```

### Usage

```ts
import { screenRec } from "@tynd/plugin-screen-rec"

const handle = await screenRec.start({
  output: "/tmp/recording.mp4",
  fps: 60,
  codec: "h264",
  encoder: "auto",   // picks NVENC / VideoToolbox / QuickSync / x264
  bitrate: 12_000,   // 12 Mbps
  preset: "fast",
})

// ...later
await screenRec.stop({ recordingId: handle.id })
```

### Encoder selection

`encoder: "auto"` walks this priority list and picks the first one
ffmpeg reports as available in your build:

| Hardware | H.264 encoder | HEVC encoder |
|---|---|---|
| NVIDIA GPU (Win / Linux) | `h264_nvenc` | `hevc_nvenc` |
| Apple silicon / Intel Mac | `h264_videotoolbox` | `hevc_videotoolbox` |
| Intel CPU with iGPU (recent) | `h264_qsv` | `hevc_qsv` |
| AMD / Intel on Linux (DRM) | `h264_vaapi` | `hevc_vaapi` |
| CPU fallback (always) | `libx264` | `libx265` |

Pinning `encoder` to a specific backend skips the fallback chain — if
the requested encoder isn't compiled into your ffmpeg, `start()`
errors immediately rather than silently downgrading. Useful for
benchmarks and CI pinning.

## Permissions

| Platform | Behaviour |
|---|---|
| **macOS** 12.3+ | Capture requires the user to grant **System Settings → Privacy & Security → Screen & System Audio Recording** for the host binary. The first `requestPermission()` call surfaces the system prompt. |
| **Windows** 10+ | No OS-level permission, but Windows 11 may surface a one-shot UAC dialog when the host attaches to the desktop composition. `hasPermission()` returns `true`. |
| **Linux** (Wayland) | Capture goes through the PipeWire portal (`xdg-desktop-portal`); the portal itself prompts the user. |
| **Linux** (X11) | No permission — capture goes straight through XCB. |

Every discovery, permission and recording command first invokes the host's
`security.checkScreenCapture` policy. Missing policy dispatch fails closed;
the OS permission remains a second independent gate.

## Limitations

- **Finalisation is bounded.** `stop()` waits up to five seconds for encoder
  finalisation. A stuck OS capture call is detached with a warning rather than
  freezing the host.
- **Audio capture is not implemented.** `audio: true` is rejected instead of
  being silently ignored.
- `pause` / `resume` flip an encoder-side flag that drops frames —
  there's no native `scap` pause in 0.0.8, so the capture device
  keeps running in the background. Wall-clock duration is correctly
  discounted in the `RecordingReport`.
- `scap` 0.0.8 does not expose a window's owning application, so
  `WindowInfo.appName` is currently an empty string.
- Linux capture portals select a source interactively and do not expose target
  IDs before capture; `listSources()` therefore returns empty arrays there and
  `monitor` must be omitted.
- Region capture is supported; dimensions are validated and the H.264 scaler
  is initialised from the first real frame.

## Why a plugin?

Screen recording is a heavy, OS-specific niche feature with a large
native dep footprint. Keeping it out of the preset runtimes honours
the "API = OS primitive, plugin = niche feature" rule and lets apps
that don't need recording skip the entire `scap` build chain.

[scap]: https://github.com/CapSoftware/scap

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
