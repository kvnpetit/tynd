import { base64ToBytes, bytesToBase64, osCall } from "./_internal.ts"

export interface WebSocketMessage {
  kind: "text" | "binary"
  data: string | Uint8Array
}

export interface WebSocketHandle {
  id: number
  send(data: string | Uint8Array): Promise<void>
  ping(): Promise<void>
  close(code?: number, reason?: string): Promise<void>
  onOpen(handler: () => void): () => void
  onMessage(handler: (msg: WebSocketMessage) => void): () => void
  onClose(handler: (code: number) => void): () => void
  onError(handler: (message: string) => void): () => void
}

function makeHandle(id: number): WebSocketHandle {
  const matchId = (p: unknown): p is { id: number } =>
    typeof p === "object" && p !== null && (p as { id: number }).id === id

  return {
    id,
    send(data) {
      if (typeof data === "string") {
        return osCall("websocket", "send", { id, kind: "text", data })
      }
      return osCall("websocket", "send", { id, kind: "binary", data: bytesToBase64(data) })
    },
    ping() {
      return osCall("websocket", "send", { id, kind: "ping" })
    },
    close(code, reason) {
      return osCall("websocket", "close", { id, code, reason })
    },
    onOpen(handler) {
      return window.__tynd__.os_on("websocket:open", (p) => {
        if (matchId(p)) handler()
      })
    },
    onMessage(handler) {
      return window.__tynd__.os_on("websocket:message", (p) => {
        const payload = p as { id: number; kind: "text" | "binary"; data: string }
        if (payload?.id !== id) return
        handler({
          kind: payload.kind,
          data: payload.kind === "binary" ? base64ToBytes(payload.data) : payload.data,
        })
      })
    },
    onClose(handler) {
      return window.__tynd__.os_on("websocket:close", (p) => {
        const payload = p as { id: number; code: number }
        if (payload?.id === id) handler(payload.code)
      })
    },
    onError(handler) {
      return window.__tynd__.os_on("websocket:error", (p) => {
        const payload = p as { id: number; message: string }
        if (payload?.id === id) handler(payload.message)
      })
    },
  }
}

export const websocket = {
  async connect(url: string): Promise<WebSocketHandle> {
    const { id } = await osCall<{ id: number }>("websocket", "connect", { url })
    return makeHandle(id)
  },
  list(): Promise<number[]> {
    return osCall("websocket", "list")
  },
}
