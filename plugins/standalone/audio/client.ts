/**
 * `@tynd/plugin-audio` — single-import entry. `startCapture()` returns
 * an `AsyncIterable<{ samples, timestamp }>`; `playFile` / `playSamples`
 * return playback handles.
 */

import { base64ToBytes } from "@tynd/core/client/bridge";
import {
  composePluginClient,
  createResourceCloser,
  defineTypedPlugin,
  onHostEvent,
} from "@tynd/plugin-sdk";
import type { CommandsCatalog } from "./audio.gen";

const _api = defineTypedPlugin<CommandsCatalog>("audio");

/** Capture configuration for `audio.startCapture`. */
export interface AudioCaptureOptions {
  sampleRate?: number;
  channels?: number;
  /** Output is always f32; other values are rejected. */
  format?: "f32";
  device?: string;
}
/** Single chunk emitted by an `audio.startCapture` async iterator. */
export interface AudioCaptureChunk {
  samples: Float32Array;
  /** Epoch ms when the chunk was captured by the host. */
  timestamp: number;
}
/** Options for `audio.playFile`. */
export interface AudioPlayFileOptions {
  volume?: number;
  deviceId?: string;
}
/** Options for `audio.playSamples`. */
export interface AudioPlaySamplesOptions {
  sampleRate: number;
  channels: number;
  volume?: number;
}
/** Options for `audio.saveWav`. */
export interface AudioSaveWavOptions {
  sampleRate: number;
  channels: number;
}
/** Handle to an in-flight playback; `stop()` halts it on the host side. */
export interface AudioPlayHandle {
  id: number;
  durationMs?: number;
  stop(): Promise<void>;
}

/**
 * `audio:samples` event payload — base64-encoded little-endian f32
 * chunk emitted by every active capture stream.
 */
export interface AudioSamplesEvent {
  id: number;
  samples: string;
  timestamp: number;
}

/**
 * `audio:warn` event payload — non-fatal capture negotiation issues.
 */
export interface AudioWarnEvent {
  id: number;
  kind: string;
  requested?: number;
  actual?: number;
}

/**
 * `audio:error` event payload — OS-level stream error. Stream is dead
 * after this event; callers must `stopCapture` and re-`startCapture`.
 */
export interface AudioErrorEvent {
  id: number;
  kind: string;
  message: string;
}

function decodeF32(base64: string): Float32Array {
  const bytes = base64ToBytes(base64);
  const buf = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buf).set(bytes);
  return new Float32Array(buf);
}

type CaptureStream = AsyncIterable<AudioCaptureChunk> & {
  streamId: Promise<number>;
  stop(): Promise<void>;
};

type PendingChunk = (chunk: AudioCaptureChunk | null) => void;

function toCaptureOpts(opts: AudioCaptureOptions | undefined) {
  if (!opts) return null;
  return {
    sampleRate: opts.sampleRate ?? null,
    channels: opts.channels ?? null,
    format: opts.format ?? null,
    device: opts.device ?? null,
  };
}

/** Max queued capture chunks (~10 s at 100 chunks/s) before the oldest are dropped. */
const MAX_QUEUED_CHUNKS = 1024;
/** Max events held while the stream id is still unknown. */
const MAX_EARLY_EVENTS = 256;

type CaptureEvent = { kind: "samples" | "error"; payload: Record<string, unknown> | null };

class AudioCaptureStream implements AsyncIterable<AudioCaptureChunk> {
  readonly streamId: Promise<number>;
  private readonly queue: AudioCaptureChunk[] = [];
  private readonly close: () => Promise<void>;
  private pendingResolve: PendingChunk | null = null;
  private readonly detach: () => void;
  private failure: { error: unknown } | null = null;
  private stopped = false;
  private id: number | null = null;
  private early: CaptureEvent[] = [];

  constructor(opts: AudioCaptureOptions | undefined) {
    // Subscribe before starting so samples emitted between the native start
    // and the id reply are buffered instead of lost.
    const offSamples = onHostEvent<Record<string, unknown> | null>("audio:samples", (payload) =>
      this.onEvent({ kind: "samples", payload }),
    );
    const offError = onHostEvent<Record<string, unknown> | null>("audio:error", (payload) =>
      this.onEvent({ kind: "error", payload }),
    );
    this.detach = () => {
      offSamples();
      offError();
    };
    this.streamId = _api.startCapture({ opts: toCaptureOpts(opts) }).then(({ id }) => id);
    this.streamId.then(
      (id) => this.attachEvents(id),
      (error) => this.failStart(error),
    );
    this.close = createResourceCloser(
      () => this.stopRemote(),
      () => this.finish(),
    );
  }

  stop(): Promise<void> {
    return this.close();
  }

  [Symbol.asyncIterator](): AsyncIterator<AudioCaptureChunk> {
    return {
      next: () => this.next(),
      return: async () => {
        await this.stop();
        return { value: undefined, done: true };
      },
    };
  }

  private onEvent(event: CaptureEvent): void {
    if (this.stopped || event.payload == null) return;
    if (this.id === null) {
      this.early.push(event);
      if (this.early.length > MAX_EARLY_EVENTS) this.early.shift();
      return;
    }
    if (event.payload.id !== this.id) return;
    this.dispatch(event);
  }

  private dispatch({ kind, payload }: CaptureEvent): void {
    if (!payload) return;
    if (kind === "samples") {
      if (typeof payload.samples !== "string") return;
      this.enqueue({
        samples: decodeF32(payload.samples),
        timestamp: typeof payload.timestamp === "number" ? payload.timestamp : Date.now(),
      });
      return;
    }
    this.failDead(
      new Error(
        `audio capture stream died${typeof payload.message === "string" ? `: ${payload.message}` : ""}`,
      ),
    );
  }

  private attachEvents(id: number): void {
    if (this.stopped) return;
    this.id = id;
    const early = this.early;
    this.early = [];
    for (const event of early) {
      if (event.payload?.id === id) this.dispatch(event);
    }
  }

  private enqueue(chunk: AudioCaptureChunk): void {
    if (!this.pendingResolve) {
      this.queue.push(chunk);
      if (this.queue.length > MAX_QUEUED_CHUNKS) this.queue.shift();
      return;
    }
    const resolve = this.pendingResolve;
    this.pendingResolve = null;
    resolve(chunk);
  }

  private failStart(error: unknown): void {
    this.failure = { error };
    this.finish();
  }

  /** The native stream reported a fatal device error: end the iterator with it. */
  private failDead(error: unknown): void {
    if (this.stopped) return;
    this.failure = { error };
    this.finish();
    // Release the dead native stream; it may already be gone.
    this.close().catch(() => undefined);
  }

  private async stopRemote(): Promise<void> {
    const id = await this.streamId.catch(() => null);
    if (id != null) await _api.stopCapture({ id });
  }

  private finish(): void {
    this.stopped = true;
    this.early = [];
    this.detach();
    this.resolvePending(null);
  }

  private resolvePending(chunk: AudioCaptureChunk | null): void {
    const resolve = this.pendingResolve;
    this.pendingResolve = null;
    resolve?.(chunk);
  }

  private throwIfFailed(): void {
    if (this.failure) throw this.failure.error;
  }

  private async next(): Promise<IteratorResult<AudioCaptureChunk>> {
    // Deliver what was captured before a failure, then surface the failure.
    const queued = this.queue.shift();
    if (queued) return { value: queued, done: false };
    this.throwIfFailed();
    if (this.stopped) return { value: undefined, done: true };

    const chunk = await new Promise<AudioCaptureChunk | null>((resolve) => {
      this.pendingResolve = resolve;
    });
    this.throwIfFailed();
    return chunk ? { value: chunk, done: false } : { value: undefined, done: true };
  }
}

/** Frontend bindings for the `@tynd/plugin-audio` plugin (capture, playback, file IO). */
export const audio = composePluginClient(_api, {
  /** Enumerate available audio input and output device names. */
  listDevices(): Promise<{ inputs: string[]; outputs: string[] }> {
    return _api.listDevices() as Promise<unknown> as Promise<{
      inputs: string[];
      outputs: string[];
    }>;
  },

  /** Start a capture stream; iterate chunks as they arrive. Call `stop()` or break to end. */
  startCapture(opts?: AudioCaptureOptions): CaptureStream {
    return new AudioCaptureStream(opts);
  },
  /** Stop an active capture stream by its id. */
  async stopCapture(streamId: number): Promise<void> {
    await _api.stopCapture({ id: streamId });
  },

  /** Play an audio file from disk; resolves to a handle that controls the playback. */
  async playFile(path: string, opts?: AudioPlayFileOptions): Promise<AudioPlayHandle> {
    const wireOpts = {
      volume: opts?.volume ?? null,
      deviceId: opts?.deviceId ?? null,
    };
    const r = (await _api.playFile({ path, opts: wireOpts })) as {
      id: number;
      durationMs?: number;
    };
    return {
      id: r.id,
      durationMs: r.durationMs,
      stop: () => _api.stopPlayback({ id: r.id }) as Promise<void>,
    };
  },

  /** Play raw float32 samples; resolves to a handle that controls the playback. */
  async playSamples(
    samples: Float32Array,
    opts: AudioPlaySamplesOptions,
  ): Promise<AudioPlayHandle> {
    const arr = Array.from(samples);
    const wireOpts = {
      sampleRate: opts.sampleRate,
      channels: opts.channels,
      volume: opts.volume ?? null,
    };
    const r = (await _api.playSamples({ samples: arr, opts: wireOpts })) as {
      id: number;
    };
    return {
      id: r.id,
      stop: () => _api.stopPlayback({ id: r.id }) as Promise<void>,
    };
  },

  /** Stop an active playback by id. */
  async stopPlayback(id: number): Promise<void> {
    await _api.stopPlayback({ id });
  },

  /** Save raw samples as a WAV file at the given path. */
  async saveWav(samples: Float32Array, path: string, opts: AudioSaveWavOptions): Promise<void> {
    const wireOpts = { sampleRate: opts.sampleRate, channels: opts.channels };
    await _api.saveWav({
      samples: Array.from(samples),
      path,
      opts: wireOpts,
    });
  },
});

export default audio;
