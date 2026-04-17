declare global {
  interface Window {
    __tynd__: {
      call(fn: string, args: unknown[]): Promise<unknown>
      os_call(api: string, method: string, args: unknown): Promise<unknown>
      os_on(name: string, handler: (data: unknown) => void): () => void
      on(name: string, handler: (payload: unknown) => void): () => void
      off(name: string, handler: (payload: unknown) => void): void
    }
    __tynd_os_result__: (id: string, ok: boolean, value: unknown) => void
    __tynd_os_event__: (name: string, data: unknown) => void
  }
}

export function osCall<T>(api: string, method: string, args: unknown = null): Promise<T> {
  return window.__tynd__.os_call(api, method, args) as Promise<T>
}

export function bytesToBase64(bytes: Uint8Array): string {
  let s = ""
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]!)
  return btoa(s)
}

export function base64ToBytes(b64: string): Uint8Array {
  const s = atob(b64)
  const out = new Uint8Array(s.length)
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i)
  return out
}
