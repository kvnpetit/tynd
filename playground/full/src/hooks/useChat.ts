import { useCallback, useRef, useState } from "react"
import { api, type ChatMessage } from "../api"
import type { UiMessage } from "../types"

const SYSTEM_PROMPT = "You are a concise, helpful assistant. Answer clearly in plain text."

export interface UseChatApi {
  messages: UiMessage[]
  streaming: boolean
  send(input: string): Promise<void>
  stop(): void
  reset(): void
}

/**
 * Owns the conversation state + the streaming lifecycle. Splits state updates
 * so React re-renders incrementally as tokens arrive, and keeps the cancel
 * handle in a ref so `stop()` doesn't depend on stale closures.
 */
export function useChat(baseUrl: string, model: string): UseChatApi {
  const [messages, setMessages] = useState<UiMessage[]>([])
  const [streaming, setStreaming] = useState(false)
  const cancelRef = useRef<(() => Promise<unknown>) | null>(null)

  const send = useCallback(
    async (input: string) => {
      const prompt = input.trim()
      if (!prompt || streaming) return

      const history: UiMessage[] = [...messages, { role: "user", content: prompt }]
      setMessages([...history, { role: "assistant", content: "", streaming: true }])
      setStreaming(true)

      const payload: ChatMessage[] = [
        { role: "system", content: SYSTEM_PROMPT },
        ...history.map((m) => ({ role: m.role, content: m.content }) as ChatMessage),
      ]
      const handle = api.chat(payload, model ? { baseUrl, model } : { baseUrl })
      cancelRef.current = () => handle.cancel()

      let accumulated = ""
      try {
        for await (const delta of handle) {
          accumulated += delta
          setMessages((prev) => {
            const next = [...prev]
            next[next.length - 1] = {
              role: "assistant",
              content: accumulated,
              streaming: true,
            }
            return next
          })
        }
        setMessages((prev) => {
          const next = [...prev]
          next[next.length - 1] = { role: "assistant", content: accumulated }
          return next
        })
      } catch (e) {
        setMessages((prev) => {
          const next = [...prev]
          next[next.length - 1] = {
            role: "assistant",
            content: accumulated
              ? `${accumulated}\n\n[stopped: ${String(e)}]`
              : `Error: ${String(e)}`,
          }
          return next
        })
      } finally {
        setStreaming(false)
        cancelRef.current = null
      }
    },
    [baseUrl, model, messages, streaming],
  )

  const stop = useCallback(() => {
    cancelRef.current?.()
  }, [])

  const reset = useCallback(() => {
    if (streaming) cancelRef.current?.()
    setMessages([])
  }, [streaming])

  return { messages, streaming, send, stop, reset }
}
