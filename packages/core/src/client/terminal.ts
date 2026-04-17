import { base64ToBytes, bytesToBase64, osCall } from "./_internal.js"

export interface TerminalSpawnOptions {
  shell?: string
  args?: string[]
  cwd?: string
  env?: Record<string, string>
  cols?: number
  rows?: number
}

export interface TerminalHandle {
  id: number
  write(data: string | Uint8Array): Promise<void>
  resize(cols: number, rows: number): Promise<void>
  kill(): Promise<void>
  onData(handler: (chunk: Uint8Array) => void): () => void
  onExit(handler: (code: number | null) => void): () => void
}

function makeHandle(id: number): TerminalHandle {
  return {
    id,
    write(data) {
      const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data
      return osCall("terminal", "write", { id, data: bytesToBase64(bytes) })
    },
    resize(cols, rows) {
      return osCall("terminal", "resize", { id, cols, rows })
    },
    kill() {
      return osCall("terminal", "kill", { id })
    },
    onData(handler) {
      return window.__tynd__.os_on("terminal:data", (payload: unknown) => {
        const p = payload as { id: number; data: string }
        if (p?.id === id) handler(base64ToBytes(p.data))
      })
    },
    onExit(handler) {
      return window.__tynd__.os_on("terminal:exit", (payload: unknown) => {
        const p = payload as { id: number; code: number | null }
        if (p?.id === id) handler(p.code)
      })
    },
  }
}

export const terminal = {
  async spawn(opts?: TerminalSpawnOptions): Promise<TerminalHandle> {
    const { id } = await osCall<{ id: number }>("terminal", "spawn", opts ?? {})
    return makeHandle(id)
  },
  list(): Promise<number[]> {
    return osCall("terminal", "list")
  },
}
