import { app } from "@tynd/core"

export interface ChatMessage {
  role: "system" | "user" | "assistant"
  content: string
}

export interface ChatOptions {
  /** Base URL of the OpenAI-compatible server. Defaults to `http://localhost:13305`. */
  baseUrl?: string
  /** Model name sent to the server. Most local runners ignore this. */
  model?: string
  /** Sampling temperature. 0 = deterministic, 1 = creative. */
  temperature?: number
  /** Hard cap on tokens generated per reply. */
  maxTokens?: number
}

const DEFAULT_BASE_URL = "http://localhost:13305"

function chatUrl(baseUrl: string): string {
  return `${baseUrl.replace(/\/+$/, "")}/api/v1/chat/completions`
}

function modelsUrl(baseUrl: string): string {
  return `${baseUrl.replace(/\/+$/, "")}/api/v1/models`
}

/**
 * Stream an LLM response token-by-token from any OpenAI-compatible server.
 * Wired through Tynd's streaming RPC — the frontend iterates with `for await`,
 * yields arrive as they're decoded from the server's SSE. Calling `.cancel()`
 * on the frontend handle aborts the underlying fetch so the upstream doesn't
 * keep generating after the user stops.
 */
export async function* chat(
  history: ChatMessage[],
  options: ChatOptions = {},
): AsyncGenerator<string, void, void> {
  const baseUrl = options.baseUrl ?? DEFAULT_BASE_URL
  const controller = new AbortController()

  try {
    const body: Record<string, unknown> = {
      messages: history,
      temperature: options.temperature ?? 0.7,
      max_tokens: options.maxTokens ?? 1024,
      stream: true,
    }
    // Only set `model` when the caller has picked one — some servers reject
    // an invented default, others let you omit the field entirely.
    if (options.model) body.model = options.model

    const res = await fetch(chatUrl(baseUrl), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
      signal: controller.signal,
    })

    if (!res.ok) {
      const err = await res.text().catch(() => res.statusText)
      throw new Error(`${res.status}: ${err}`)
    }
    if (!res.body) throw new Error("response had no body")

    const reader = res.body.getReader()
    const decoder = new TextDecoder()
    let buffer = ""

    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buffer += decoder.decode(value, { stream: true })

        // SSE frames are separated by blank lines; most servers emit one
        // `data:` line per chunk so splitting on \n is enough.
        const lines = buffer.split("\n")
        buffer = lines.pop() ?? ""

        for (const raw of lines) {
          const line = raw.trim()
          if (!line.startsWith("data:")) continue
          const payload = line.slice(5).trim()
          if (payload === "[DONE]") return

          let json: unknown
          try {
            json = JSON.parse(payload)
          } catch {
            continue
          }
          const delta = (json as { choices?: { delta?: { content?: string } }[] }).choices?.[0]
            ?.delta?.content
          if (typeof delta === "string" && delta.length > 0) yield delta
        }
      }
    } finally {
      reader.releaseLock()
    }
  } finally {
    controller.abort()
  }
}

/** Health check — frontend uses this to gate the "ready to chat" indicator. */
export async function pingServer(baseUrl?: string): Promise<{ ok: boolean; error?: string }> {
  const url = modelsUrl(baseUrl ?? DEFAULT_BASE_URL)
  try {
    const res = await fetch(url, { method: "GET" })
    return { ok: res.ok }
  } catch (e) {
    return { ok: false, error: String(e) }
  }
}

/**
 * List currently-loaded models reported by the server. Default GET (no query)
 * typically returns only the models actually available in memory — add
 * `?show_all=true` to see every model known to the server, loaded or not.
 */
export async function listModels(
  baseUrl?: string,
  showAll = false,
): Promise<{ ok: true; models: string[] } | { ok: false; error: string }> {
  const url = modelsUrl(baseUrl ?? DEFAULT_BASE_URL) + (showAll ? "?show_all=true" : "")
  try {
    const res = await fetch(url, { method: "GET" })
    if (!res.ok) {
      const text = await res.text().catch(() => res.statusText)
      return { ok: false, error: `${res.status}: ${text || res.statusText}` }
    }
    const json = (await res.json()) as { data?: { id?: string }[] }
    const models = (json.data ?? [])
      .map((m) => m.id)
      .filter((id): id is string => typeof id === "string" && id.length > 0)
    return { ok: true, models }
  } catch (e) {
    return { ok: false, error: String(e) }
  }
}

app.start({
  frontendDir: `${import.meta.dir}/../dist`,
  window: {
    title: "Tynd Chat",
    width: 960,
    height: 720,
    minWidth: 480,
    minHeight: 400,
    center: true,
  },
})
