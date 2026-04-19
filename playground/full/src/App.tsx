import { useMemo } from "react"
import { baseUrlForPort } from "./api"
import { ChatHeader } from "./components/ChatHeader"
import { ChatInput } from "./components/ChatInput"
import { MessageList } from "./components/MessageList"
import { useChat } from "./hooks/useChat"
import { useModels } from "./hooks/useModels"
import { usePersistedModel } from "./hooks/usePersistedModel"
import { usePersistedPort } from "./hooks/usePersistedPort"
import { useServerStatus } from "./hooks/useServerStatus"
import "./App.css"

export default function App() {
  const [port, setPort] = usePersistedPort()
  const baseUrl = useMemo(() => baseUrlForPort(port), [port])
  const status = useServerStatus(baseUrl)
  const { models, error: modelsError } = useModels(baseUrl)
  const [model, setModel] = usePersistedModel(baseUrl, models)
  const { messages, streaming, send, stop, reset } = useChat(baseUrl, model)

  return (
    <div className="chat">
      <ChatHeader
        status={status}
        baseUrl={baseUrl}
        port={port}
        onPortChange={setPort}
        models={models}
        modelsError={modelsError}
        model={model}
        onModelChange={setModel}
        onReset={reset}
        canReset={messages.length > 0 || streaming}
        streaming={streaming}
      />
      <MessageList messages={messages} baseUrl={baseUrl} />
      <ChatInput streaming={streaming} onSend={send} onStop={stop} />
    </div>
  )
}
