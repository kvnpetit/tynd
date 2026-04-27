import { osCall } from "./_internal"

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
    /** `true` when `key` is present, regardless of whether its value is `null`. */
    has(key: string): Promise<boolean> {
      return osCall("store", "has", { namespace, key }) as Promise<boolean>
    },
    clear(): Promise<void> {
      return osCall("store", "clear", { namespace })
    },
    keys(): Promise<string[]> {
      return osCall("store", "keys", { namespace })
    },
    values<T = unknown>(): Promise<T[]> {
      return osCall("store", "values", { namespace }) as Promise<T[]>
    },
    /** `[key, value][]` pairs — feed straight into `Object.fromEntries`. */
    entries<T = unknown>(): Promise<Array<[string, T]>> {
      return osCall("store", "entries", { namespace }) as Promise<Array<[string, T]>>
    },
    /** Number of keys currently stored. */
    length(): Promise<number> {
      return osCall("store", "length", { namespace }) as Promise<number>
    },
    /**
     * Drop the in-memory cache for this namespace and re-read `store.json`
     * from disk. Useful when another process (a sibling app instance, a
     * manual edit) mutated the file after this process started.
     */
    reload(): Promise<void> {
      return osCall("store", "reload", { namespace })
    },
  }
}
