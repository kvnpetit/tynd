/**
 * Vercel AI SDK adapter for `@tynd/plugin-llm`.
 * Implements the `LanguageModelV3` interface from `@ai-sdk/provider` so
 * consumers can plug local llama.cpp models into the broader Vercel AI
 * SDK ecosystem (`streamText`, `generateText`, `useChat()` React/Vue/
 * Svelte/Solid hooks, conversation state, abort signals, etc.) without
 * rewriting any glue themselves.
 * Peer dependencies (install yourself if you use this adapter):
 *   - `ai` (the Vercel AI SDK runtime, v6+)
 *   - `@ai-sdk/provider` (the LanguageModelV3 interface types, v3+)
 * @example
 * ```ts
 * import { llm } from "@tynd/plugin-llm/client"
 * import { tyndLlmProvider } from "@tynd/plugin-llm/ai-sdk"
 * import { streamText } from "ai"
 * const model = await llm.load({ path: "./mistral-7b.gguf" })
 * const result = streamText({
 *   model: tyndLlmProvider(model.id, { name: "mistral-7b" }),
 *   messages: [{ role: "user", content: "Hello" }],
 * })
 * for await (const chunk of result.textStream) process.stdout.write(chunk)
 * ```
 */

import type {
  LanguageModelV3,
  LanguageModelV3CallOptions,
  LanguageModelV3FinishReason,
  LanguageModelV3Prompt,
  LanguageModelV3StreamPart,
} from "@ai-sdk/provider";
import { type GenerateOptions, llm } from "./client";

export interface TyndLlmProviderOptions {
  /** Display name reported as `modelId` (defaults to `tynd-llm-<id>`). */
  name?: string;
}

/**
 * Wraps a loaded Tynd LLM model handle as a Vercel AI SDK
 * `LanguageModelV3`.
 * @param modelId Handle returned by `llm.load(...)` — see `LlmModel.id`.
 * @param opts   Optional overrides (e.g. friendly model name).
 */
export function tyndLlmProvider(
  modelId: number,
  opts: TyndLlmProviderOptions = {},
): LanguageModelV3 {
  return {
    specificationVersion: "v3",
    provider: "tynd-llm",
    modelId: opts.name ?? `tynd-llm-${modelId}`,
    supportedUrls: {},
    doGenerate: (options) => generateWithModel(modelId, options),
    doStream: (options) => streamWithModel(modelId, options),
  };
}

async function generateWithModel(modelId: number, options: LanguageModelV3CallOptions) {
  const prompt = formatPrompt(options.prompt);
  const generateOpts = mapOptions(options);
  const text = await llm.complete(modelId, prompt, generateOpts);

  return {
    content: [{ type: "text" as const, text }],
    finishReason: makeFinishReason("stop"),
    usage: makeUsage(),
    warnings: collectWarnings(options),
    request: { body: { prompt, options: generateOpts } },
  };
}

async function streamWithModel(modelId: number, options: LanguageModelV3CallOptions) {
  const prompt = formatPrompt(options.prompt);
  const generateOpts = mapOptions(options);
  const stream = createModelStream(modelId, prompt, generateOpts, options);
  return {
    stream,
    request: { body: { prompt, options: generateOpts } },
  };
}

function createModelStream(
  modelId: number,
  prompt: string,
  generateOpts: GenerateOptions,
  options: LanguageModelV3CallOptions,
): ReadableStream<LanguageModelV3StreamPart> {
  const warnings = collectWarnings(options);
  const textBlockId = `txt-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  return new ReadableStream({
    async start(controller) {
      controller.enqueue({ type: "stream-start", warnings });
      controller.enqueue({ type: "text-start", id: textBlockId });
      try {
        for await (const token of llm.generate(modelId, prompt, {
          ...generateOpts,
          signal: options.abortSignal,
        })) {
          controller.enqueue({ type: "text-delta", id: textBlockId, delta: token });
        }
        finishStream(controller, textBlockId, "stop");
      } catch (error) {
        controller.enqueue({ type: "text-end", id: textBlockId });
        controller.enqueue({ type: "error", error });
        controller.enqueue({
          type: "finish",
          finishReason: makeFinishReason("error"),
          usage: makeUsage(),
        });
      } finally {
        controller.close();
      }
    },
  });
}

function finishStream(
  controller: ReadableStreamDefaultController<LanguageModelV3StreamPart>,
  textBlockId: string,
  reason: LanguageModelV3FinishReason["unified"],
): void {
  controller.enqueue({ type: "text-end", id: textBlockId });
  controller.enqueue({
    type: "finish",
    finishReason: makeFinishReason(reason),
    usage: makeUsage(),
  });
}

type PromptMessage = LanguageModelV3Prompt[number];
type ToolPromptMessage = Extract<PromptMessage, { role: "tool" }>;
type TextPromptMessage = Exclude<PromptMessage, { role: "system" | "tool" }>;

function toolMessageText(message: ToolPromptMessage): string {
  const chunks: string[] = [];
  for (const part of message.content) {
    if (part.type !== "tool-result") continue;
    const output = part.output;
    if (output.type === "text" || output.type === "error-text") {
      chunks.push(output.value);
    } else if (output.type === "json" || output.type === "error-json") {
      chunks.push(JSON.stringify(output.value));
    }
  }
  return chunks.join("\n");
}

function textMessageText(message: TextPromptMessage): string {
  const chunks: string[] = [];
  for (const part of message.content) {
    if (part.type === "text") chunks.push(part.text);
  }
  return chunks.join("");
}

function promptMessageText(message: PromptMessage): string {
  if (message.role === "system") return message.content;
  if (message.role === "tool") return toolMessageText(message);
  return textMessageText(message);
}

/**
 * Flatten an AI SDK `LanguageModelV3Prompt` into a single string the
 * llama.cpp engine can tokenise. We use a minimal `role: content` per
 * line, terminated with `assistant:` so the model continues from the
 * assistant turn. Models with chat-template metadata baked into the
 * GGUF (Mistral, Llama-3 ChatML, Qwen, …) usually still produce
 * coherent output with this scheme — for stricter control you can
 * pre-format the prompt yourself and call `llm.generate` directly.
 * Multimodal file parts (images/PDFs) are silently dropped — the
 * llama.cpp text-only path doesn't consume them.
 */
function formatPrompt(prompt: LanguageModelV3Prompt): string {
  const parts: string[] = [];
  for (const message of prompt) {
    const text = promptMessageText(message);
    if (text) parts.push(`${message.role}: ${text}`);
  }
  parts.push("assistant:");
  return parts.join("\n");
}

/** Map AI SDK call options to Tynd llm `GenerateOptions`. */
function mapOptions(options: LanguageModelV3CallOptions): GenerateOptions {
  return {
    maxTokens: options.maxOutputTokens,
    temperature: options.temperature,
    topP: options.topP,
    topK: options.topK,
    seed: options.seed,
    stop: options.stopSequences,
  };
}

type GenerateResult = Awaited<ReturnType<LanguageModelV3["doGenerate"]>>;
type CallWarnings = NonNullable<GenerateResult["warnings"]>;

/**
 * Surface unsupported settings as standard AI SDK warnings rather than
 * silently dropping them — gives the consumer a clear signal that e.g.
 * tool calling won't work with this engine.
 */
function collectWarnings(options: LanguageModelV3CallOptions): CallWarnings {
  const warnings: CallWarnings = [];
  if (options.tools && options.tools.length > 0) {
    warnings.push({
      type: "unsupported",
      feature: "tools",
      details:
        "Tool calling is not supported by the llama.cpp engine. Use a tool-finetuned model and parse JSON yourself.",
    });
  }
  if (options.responseFormat?.type === "json") {
    warnings.push({
      type: "unsupported",
      feature: "responseFormat",
      details:
        "Structured JSON output is not natively constrained — prompt the model for JSON instead.",
    });
  }
  if (options.frequencyPenalty != null) {
    warnings.push({ type: "unsupported", feature: "frequencyPenalty" });
  }
  if (options.presencePenalty != null) {
    warnings.push({ type: "unsupported", feature: "presencePenalty" });
  }
  return warnings;
}

/** Build a `LanguageModelV3FinishReason` from the unified label. */
function makeFinishReason(
  unified: LanguageModelV3FinishReason["unified"],
): LanguageModelV3FinishReason {
  return { unified, raw: undefined };
}

/**
 * Empty-but-typed usage block. The llama.cpp plugin doesn't expose
 * prompt/completion token counts yet; fill these in once the engine does.
 */
function makeUsage(): GenerateResult["usage"] {
  return {
    inputTokens: {
      total: undefined,
      noCache: undefined,
      cacheRead: undefined,
      cacheWrite: undefined,
    },
    outputTokens: {
      total: undefined,
      text: undefined,
      reasoning: undefined,
    },
  };
}
