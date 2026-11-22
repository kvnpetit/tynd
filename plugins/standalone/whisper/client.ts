/**
 * `@tynd/plugin-whisper` — single-import entry. Streaming transcription
 * via `whisper:segment` + `whisper:end` events. Whisper expects 16 kHz
 * mono f32 PCM; callers always pass `Float32Array` (base64 internal).
 */

import { bytesToBase64 } from "@tynd/core/client/bridge";
import {
  AsyncEventQueue,
  composePluginClient,
  createHandleEvents,
  defineTypedPlugin,
} from "@tynd/plugin-sdk";
import type { CommandsCatalog } from "./whisper.gen";

/** Defaults: auto-detect language, greedy sampling, no translation. */
export interface TranscribeOptions {
  /** `"auto"` (or omitted) ⇒ detect; else a BCP-47 short code. */
  language?: string;
  translate?: boolean;
  /** `0` ⇒ `available_parallelism()`. */
  threads?: number;
  /** Biases vocabulary / formatting. */
  prompt?: string;
  /** `0.0` ⇒ greedy. */
  temperature?: number;
  maxLen?: number;
  wordTimestamps?: boolean;
}

export interface WhisperModel {
  id: number;
  path: string;
  /** Always `16000`. */
  sampleRate: number;
  /** `"multi"` for multilingual; BCP-47 short code for monolingual. */
  language: string;
}

export interface TranscriptWord {
  text: string;
  startMs: number;
  endMs: number;
  /** Token probability in [0, 1]. */
  confidence: number;
}

/** One finalised transcript segment (event payload + transcribe yield). */
export interface TranscriptSegment {
  startMs: number;
  endMs: number;
  text: string;
  words?: TranscriptWord[];
}

export interface DetectLanguageResult {
  /** BCP-47 short code. */
  language: string;
  /** Probability of the detected language in [0, 1] (from whisper's language head). */
  confidence: number;
}

/** Per-token timing + confidence inside a segment. */
export type WordTimestamp = TranscriptWord;

interface SegmentEvent {
  stream: number;
  segment: TranscriptSegment;
}

interface EndEvent {
  stream: number;
  ok: boolean;
  error?: string;
}

export type LanguageDetection = DetectLanguageResult;

const _api = defineTypedPlugin<CommandsCatalog>("whisper");

const WHISPER_SAMPLE_RATE = 16_000;
const DEFAULT_STREAM_WINDOW_MS = 30_000;
const MAX_STREAM_WINDOW_MS = 30_000;

function streamWindowSampleCount(windowMs: number): number {
  if (!Number.isFinite(windowMs) || windowMs <= 0 || windowMs > MAX_STREAM_WINDOW_MS) {
    throw new RangeError(
      `whisper.transcribeStream: windowMs must be finite and in (0, ${MAX_STREAM_WINDOW_MS}]`,
    );
  }
  const samples = Math.floor((WHISPER_SAMPLE_RATE * windowMs) / 1000);
  if (samples < 1) {
    throw new RangeError("whisper.transcribeStream: windowMs is too small for one 16 kHz sample");
  }
  return samples;
}

/** Frontend-extended `TranscribeOptions` carrying an optional `AbortSignal`
 *  plus optional source-format hints. */
export interface ClientTranscribeOptions extends TranscribeOptions {
  signal?: AbortSignal;
  /** Source sample rate Hz; omit to let Rust default (16 kHz) apply. */
  sampleRate?: number;
  /** Source channel count; omit to let Rust default (1) apply. */
  channels?: number;
}

function encodeSamples(samples: Float32Array): string {
  const view = new Uint8Array(samples.buffer, samples.byteOffset, samples.byteLength);
  return bytesToBase64(view);
}

function cancelTranscription(stream: number): Promise<void> {
  return _api.cancel({ streamId: stream }).catch(() => undefined);
}

/** Frontend bindings for the `@tynd/plugin-whisper` plugin (streaming speech-to-text). */
export const whisper = composePluginClient(_api, {
  /** Load a `.bin` whisper model on the host; resolves to its handle. */
  async load(opts: { path: string; gpu?: boolean }): Promise<WhisperModel> {
    return _api.load({
      path: opts.path,
      gpu: opts.gpu ?? true,
    });
  },

  /** Transcribe a finite buffer; yields segments as the host emits them. */
  async *transcribe(
    modelId: number,
    samples: Float32Array,
    opts?: ClientTranscribeOptions,
  ): AsyncIterable<TranscriptSegment> {
    const { signal, sampleRate, channels, ...nativeOpts } = opts ?? {};
    const { stream } = await _api.transcribe({
      modelId,
      samples: encodeSamples(samples),
      sampleRate: sampleRate ?? null,
      channels: channels ?? null,
      opts: nativeOpts,
    });
    const queue = new AsyncEventQueue<TranscriptSegment>();

    const events = createHandleEvents(stream, "whisper-stream");
    const onAbort = (): void => {
      queue.finish();
      void cancelTranscription(stream);
    };
    const shouldStart = !signal?.aborted;

    try {
      events.on<SegmentEvent>("whisper:segment", "stream", (event) => {
        queue.push(event.segment);
      });
      events.on<EndEvent>("whisper:end", "stream", (event) => {
        queue.finish(event.ok ? undefined : (event.error ?? "whisper: transcription failed"));
      });
      if (shouldStart) signal?.addEventListener("abort", onAbort, { once: true });
      else onAbort();

      // Listeners exist before this explicit release. Native worker cannot
      // emit `whisper:segment` / `whisper:end` while its one-shot start gate is closed.
      if (shouldStart) await _api.startStream({ streamId: stream });
      yield* queue;
    } finally {
      events.disposeAll();
      if (!queue.done) await cancelTranscription(stream);
      signal?.removeEventListener("abort", onAbort);
    }
  },

  /** Convenience wrapper: collect every segment's text and return as a single string. */
  async transcribeAll(
    modelId: number,
    samples: Float32Array,
    opts?: ClientTranscribeOptions,
  ): Promise<string> {
    if (!opts?.signal) {
      const { signal: _, sampleRate, channels, ...rest } = opts ?? ({} as ClientTranscribeOptions);
      return _api.transcribeAll({
        modelId,
        samples: encodeSamples(samples),
        sampleRate: sampleRate ?? null,
        channels: channels ?? null,
        opts: rest,
      });
    }
    let out = "";
    for await (const seg of whisper.transcribe(modelId, samples, opts)) {
      out += seg.text;
    }
    return out;
  },

  /** Buffers incoming chunks up to `windowMs` (default 30s, whisper's
   *  max) and runs one inference pass per window. No VAD — silence is
   *  transcribed too. */
  async *transcribeStream(
    modelId: number,
    audioStream: AsyncIterable<Float32Array>,
    opts?: ClientTranscribeOptions & { windowMs?: number },
  ): AsyncIterable<TranscriptSegment> {
    const { windowMs = DEFAULT_STREAM_WINDOW_MS, ...transcribeOpts } = opts ?? {};
    const samplesPerWindow = streamWindowSampleCount(windowMs);
    const windowOffset = { value: 0 };
    let buf: Float32Array = new Float32Array(samplesPerWindow);
    let filled = 0;

    const flush = async function* (this: void): AsyncGenerator<TranscriptSegment> {
      if (filled === 0) return;
      const window = buf.subarray(0, filled);
      for await (const seg of whisper.transcribe(modelId, window, transcribeOpts)) {
        yield {
          ...seg,
          startMs: seg.startMs + windowOffset.value,
          endMs: seg.endMs + windowOffset.value,
        };
      }
      windowOffset.value += Math.floor((filled * 1000) / WHISPER_SAMPLE_RATE);
      filled = 0;
      buf = new Float32Array(samplesPerWindow);
    };

    const signal = opts?.signal;
    for await (const chunk of audioStream) {
      if (signal?.aborted) return;
      let read = 0;
      while (read < chunk.length) {
        const space = samplesPerWindow - filled;
        const take = Math.min(space, chunk.length - read);
        buf.set(chunk.subarray(read, read + take), filled);
        filled += take;
        read += take;
        if (filled === samplesPerWindow) {
          yield* flush();
          if (signal?.aborted) return;
        }
      }
    }
    if (signal?.aborted) return;
    yield* flush();
  },

  /** Run whisper's language detection pass and return the top guess + confidence. */
  async detectLanguage(
    modelId: number,
    samples: Float32Array,
    opts?: { sampleRate?: number; channels?: number },
  ): Promise<DetectLanguageResult> {
    return _api.detectLanguage({
      modelId,
      samples: encodeSamples(samples),
      sampleRate: opts?.sampleRate ?? null,
      channels: opts?.channels ?? null,
    });
  },

  /**
   * Cancel an in-flight `transcribe(...)` stream. The native decode polls
   * the cancel flag and aborts promptly; segments are only emitted once the
   * whole buffer is decoded, so a cancelled run emits none. Idempotent —
   * unknown / already-finished ids are a no-op.
   */
  async cancel(streamId: number): Promise<void> {
    await _api.cancel({ streamId });
  },

  /** Unload the model from host memory. */
  async unload(modelId: number): Promise<void> {
    await _api.unload({ modelId });
  },

  /** List every loaded model in this app. */
  list(): Promise<WhisperModel[]> {
    return _api.list();
  },
});

export default whisper;
