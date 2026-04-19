import { useEffect, useRef, useState } from "react"

interface Props {
  streaming: boolean
  onSend(text: string): void
  onStop(): void
}

const MAX_TEXTAREA_HEIGHT = 160

export function ChatInput({ streaming, onSend, onStop }: Props) {
  const [text, setText] = useState("")
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  // Auto-grow the textarea up to MAX_TEXTAREA_HEIGHT, then scroll internally.
  useEffect(() => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = `${Math.min(el.scrollHeight, MAX_TEXTAREA_HEIGHT)}px`
  }, [text])

  function submit() {
    const trimmed = text.trim()
    if (!trimmed || streaming) return
    onSend(trimmed)
    setText("")
  }

  function onKey(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault()
      submit()
    }
  }

  return (
    <footer className="chat-input">
      <textarea
        ref={textareaRef}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKey}
        placeholder={
          streaming ? "Streaming…" : "Type a message (Enter to send, Shift+Enter for newline)"
        }
        disabled={streaming}
        rows={1}
      />
      {streaming ? (
        <button type="button" className="btn btn-stop" onClick={onStop}>
          Stop
        </button>
      ) : (
        <button type="button" className="btn btn-send" onClick={submit} disabled={!text.trim()}>
          Send
        </button>
      )}
    </footer>
  )
}
