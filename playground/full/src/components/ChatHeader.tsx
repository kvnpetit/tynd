import type { ServerStatus } from "../types"
import { ModelSelect } from "./ModelSelect"
import { PortInput } from "./PortInput"
import { StatusDot } from "./StatusDot"

interface Props {
  status: ServerStatus
  baseUrl: string
  port: number
  onPortChange(port: number): void
  models: string[] | null
  modelsError: string | null
  model: string
  onModelChange(model: string): void
  onReset(): void
  canReset: boolean
  streaming: boolean
}

export function ChatHeader({
  status,
  baseUrl,
  port,
  onPortChange,
  models,
  modelsError,
  model,
  onModelChange,
  onReset,
  canReset,
  streaming,
}: Props) {
  return (
    <header className="chat-header">
      <h1>Tynd Chat</h1>
      <div className="chat-status">
        <StatusDot status={status} baseUrl={baseUrl} />
        <PortInput port={port} onCommit={onPortChange} disabled={streaming} />
        <ModelSelect
          models={models}
          value={model}
          onChange={onModelChange}
          disabled={streaming}
          error={modelsError}
        />
        <button type="button" className="chat-reset" onClick={onReset} disabled={!canReset}>
          Reset
        </button>
      </div>
    </header>
  )
}
