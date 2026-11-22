/**
 * `@tynd/plugin-screen-rec` — single-import entry. Capture state lives
 * in the Rust crate; this is a thin typed proxy. Without the `capture`
 * Cargo feature, `start()` rejects + `hasPermission()` returns `false`.
 */

import { composePluginClient, defineTypedPlugin } from "@tynd/plugin-sdk";
import type {
  Codec,
  CommandsCatalog,
  EncoderChoice,
  Preset,
  RecordingHandle,
  RecordingOpts,
  Region,
} from "./screen-rec.gen";

const _api = defineTypedPlugin<CommandsCatalog>("screen-rec");

// Re-export wire types for users.
export type {
  Codec,
  EncoderChoice,
  MonitorInfo,
  Preset,
  RecordingHandle,
  RecordingOpts,
  RecordingReport,
  Region,
  Sources,
  WindowInfo,
} from "./screen-rec.gen";

// Mirrors `#[serde(default)]` on the Rust `RecordingOpts` so callers
// only need to pass `output`. Specta-typescript@0.0.11 cannot emit `?:`
// for fields that carry serde defaults (unified mode limitation), so
// this wrapper fills them client-side.
export interface RecordingOptions {
  output: string;
  fps?: number;
  region?: Region | null;
  monitor?: number | null;
  audio?: boolean;
  format?: string | null;
  codec?: Codec;
  encoder?: EncoderChoice;
  bitrate?: number;
  preset?: Preset;
}

function fillDefaults(opts: RecordingOptions): RecordingOpts {
  return {
    fps: opts.fps ?? 30,
    region: opts.region ?? null,
    monitor: opts.monitor ?? null,
    audio: opts.audio ?? false,
    format: opts.format ?? null,
    output: opts.output,
    codec: opts.codec ?? "mjpeg",
    encoder: opts.encoder ?? "auto",
    bitrate: opts.bitrate ?? 8000,
    preset: opts.preset ?? "fast",
  };
}

/** `screen-rec` plugin: descriptor + typed RPC methods. */
export const screenRec = composePluginClient(_api, {
  /** Start a recording; only `output` is required, all other fields default to the Rust-side `#[serde(default)]` values. */
  async start(opts: RecordingOptions): Promise<RecordingHandle> {
    return (await _api.start({
      opts: fillDefaults(opts),
    })) as unknown as RecordingHandle;
  },
});

export default screenRec;
