/**
 * `@tynd/plugin-llm` — single-import entry. Wraps `llm:token` + `llm:end`
 * events into an `AsyncIterable<string>` for `for await` consumption.
 */

import {
  AsyncEventQueue,
  composePluginClient,
  createHandleEvents,
  defineTypedPlugin,
} from "@tynd/plugin-sdk";
import type { CommandsCatalog, GenerateOptions, LlmModel } from "./llm.gen";

interface TokenEvent {
  stream: number;
  token: string;
}

interface EndEvent {
  stream: number;
  ok: boolean;
  error?: string;
}

export type { GenerateOptions, LlmModel };

const _api = defineTypedPlugin<CommandsCatalog>("llm");

/** Adds an `AbortSignal` — generation finishes in background, iterator
 *  returns immediately on abort. */
export interface ClientGenerateOptions extends GenerateOptions {
  signal?: AbortSignal;
}

function cancelGeneration(stream: number): Promise<void> {
  return _api.cancel({ streamId: stream }).catch(() => undefined);
}

/** Frontend bindings for the `@tynd/plugin-llm` plugin (local llama.cpp generation). */
export const llm = composePluginClient(_api, {
  /** Load a `.gguf` model on the host; resolves to the model handle. */
  load(opts: { path: string; contextSize?: number; gpuLayers?: number }): Promise<LlmModel> {
    return _api.load({
      path: opts.path,
      contextSize: opts.contextSize ?? null,
      gpuLayers: opts.gpuLayers ?? null,
    });
  },

  /** Stream tokens for a prompt; iterator returns when host emits `llm:end` or signal aborts. */
  async *generate(
    modelId: number,
    prompt: string,
    opts?: ClientGenerateOptions,
  ): AsyncIterable<string> {
    const { signal, ...nativeOpts } = opts ?? {};
    const { stream } = await _api.generate({
      modelId,
      prompt,
      opts: nativeOpts,
    });
    const queue = new AsyncEventQueue<string>();

    const events = createHandleEvents(stream, "llm-stream");
    const onAbort = (): void => {
      queue.finish();
      void cancelGeneration(stream);
    };
    const shouldStart = !signal?.aborted;

    try {
      events.on<TokenEvent>("llm:token", "stream", (event) => queue.push(event.token));
      events.on<EndEvent>("llm:end", "stream", (event) => {
        queue.finish(event.ok ? undefined : (event.error ?? "llm: generation failed"));
      });
      if (shouldStart) signal?.addEventListener("abort", onAbort, { once: true });
      else onAbort();

      // Listeners exist before this explicit release. Native worker cannot
      // emit `llm:token` / `llm:end` while its one-shot start gate is closed.
      if (shouldStart) await _api.startStream({ streamId: stream });
      yield* queue;
    } finally {
      events.disposeAll();
      if (!queue.done) await cancelGeneration(stream);
      signal?.removeEventListener("abort", onAbort);
    }
  },

  /** Convenience wrapper: collect every generated token into a single string. */
  async complete(modelId: number, prompt: string, opts?: ClientGenerateOptions): Promise<string> {
    if (!opts?.signal) {
      const { signal: _, ...rest } = opts ?? ({} as ClientGenerateOptions);
      return _api.complete({
        modelId,
        prompt,
        opts: rest,
      });
    }
    let out = "";
    for await (const tok of llm.generate(modelId, prompt, opts)) {
      out += tok;
    }
    return out;
  },

  /**
   * Cancel an in-flight `generate(...)` stream. The host inference
   * thread notices on the next token and emits its terminating
   * `llm:end`. Idempotent — unknown / already-finished ids are a
   * no-op.
   */
  async cancel(streamId: number): Promise<void> {
    await _api.cancel({ streamId });
  },

  /** Unload the model from host memory. */
  async unload(modelId: number): Promise<void> {
    await _api.unload({ modelId });
  },

  /** List every loaded model in this app. */
  list(): Promise<LlmModel[]> {
    return _api.list();
  },
});

export default llm;
