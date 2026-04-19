import { useEffect, useRef } from "react"
import type { UiMessage } from "../types"
import { Message } from "./Message"

interface Props {
  messages: UiMessage[]
  baseUrl: string
}

export function MessageList({ messages, baseUrl }: Props) {
  const endRef = useRef<HTMLDivElement>(null)

  // Scroll the latest message into view on every update. Behaves as "pinned
  // to bottom" since new tokens push the ref down on each render.
  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" })
  }, [messages])

  return (
    <main className="chat-messages">
      {messages.length === 0 ? (
        <EmptyState baseUrl={baseUrl} />
      ) : (
        messages.map((m, i) => <Message key={i} {...m} />)
      )}
      <div ref={endRef} />
    </main>
  )
}

function EmptyState({ baseUrl }: { baseUrl: string }) {
  return (
    <div className="chat-empty">
      <p>
        Chatting with <code>{baseUrl}</code>.
      </p>
      <p className="chat-empty-hint">Ask anything. Replies stream in token-by-token.</p>
    </div>
  )
}
