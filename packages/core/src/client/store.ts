import { osCall } from "./_internal.js"

export function createStore(namespace: string) {
  return {
    get<T = unknown>(key: string): Promise<T | null> {
      return osCall("store", "get", { namespace, key }) as Promise<T | null>
    },
    set(key: string, value: unknown): Promise<void> {
      return osCall("store", "set", { namespace, key, value })
    },
    delete(key: string): Promise<void> {
      return osCall("store", "delete", { namespace, key })
    },
    clear(): Promise<void> {
      return osCall("store", "clear", { namespace })
    },
    keys(): Promise<string[]> {
      return osCall("store", "keys", { namespace })
    },
  }
}
