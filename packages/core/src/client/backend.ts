import { tynd } from "../logger.js"
import type { Emitter } from "../types.js"
import "./_internal.js"

type UnionToIntersection<U> = (U extends unknown ? (k: U) => void : never) extends (
  k: infer I,
) => void
  ? I
  : never

type ModuleEvents<T> = UnionToIntersection<
  { [K in keyof T]: T[K] extends Emitter<infer E> ? E : never }[keyof T]
>

type ModuleFunctions<T> = {
  [K in keyof T as T[K] extends (...args: infer _A) => infer _R ? K : never]: T[K] extends (
    ...args: infer A
  ) => infer R
    ? (...args: A) => Promise<Awaited<R>>
    : never
}

export type BackendClient<T> = ModuleFunctions<T> & {
  on<K extends keyof ModuleEvents<T> & string>(
    event: K,
    handler: (payload: ModuleEvents<T>[K]) => void,
  ): () => void
  once<K extends keyof ModuleEvents<T> & string>(
    event: K,
    handler: (payload: ModuleEvents<T>[K]) => void,
  ): () => void
}

/**
 * Create a fully type-safe proxy to the backend.
 *
 * @example
 * import type * as backend from "../backend/main"
 * const api = createBackend<typeof backend>()
 * await api.greet("Alice")
 */
export function createBackend<T>(): BackendClient<T> {
  return new Proxy({} as BackendClient<T>, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return undefined
      // A thenable Proxy would make `await api` invoke `api.then()` as a backend call.
      if (prop === "then" || prop === "catch" || prop === "finally") return undefined

      if (!window.__tynd__) {
        tynd.error("window.__tynd__ is not available — are you running outside a tynd app?")
        return () => Promise.reject(new Error("[tynd] not in a tynd app context"))
      }

      if (prop === "on") {
        return (event: string, handler: (p: unknown) => void) => window.__tynd__.on(event, handler)
      }

      if (prop === "once") {
        return (event: string, handler: (p: unknown) => void) => {
          let called = false
          const wrapper = (p: unknown) => {
            if (!called) {
              called = true
              window.__tynd__.off(event, wrapper)
              handler(p)
            }
          }
          window.__tynd__.on(event, wrapper)
          return () => window.__tynd__.off(event, wrapper)
        }
      }

      return (...args: unknown[]) => window.__tynd__.call(prop, args)
    },
  })
}
