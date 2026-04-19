import { useCallback, useEffect, useState } from "react"

const STORAGE_PREFIX = "tynd-chat:model:"

function keyFor(baseUrl: string): string {
  return STORAGE_PREFIX + baseUrl
}

function read(baseUrl: string): string {
  if (typeof window === "undefined") return ""
  return window.localStorage.getItem(keyFor(baseUrl)) ?? ""
}

/**
 * Selected-model state, persisted per-baseUrl so switching servers remembers
 * each one's pick. Auto-selects the first available model when the current
 * choice isn't in the list (new server, model unloaded, …).
 */
export function usePersistedModel(
  baseUrl: string,
  available: string[] | null,
): [string, (model: string) => void] {
  const [model, setModelState] = useState<string>(() => read(baseUrl))

  // Reload the persisted value whenever the server changes.
  useEffect(() => {
    setModelState(read(baseUrl))
  }, [baseUrl])

  // Snap to the first available entry if the current selection can't be
  // honored (server switched, model was unloaded, nothing saved yet).
  useEffect(() => {
    if (!available || available.length === 0) return
    if (model && available.includes(model)) return
    setModelState(available[0] ?? "")
  }, [available, model])

  const setModel = useCallback(
    (next: string) => {
      setModelState(next)
      if (typeof window !== "undefined") {
        if (next) window.localStorage.setItem(keyFor(baseUrl), next)
        else window.localStorage.removeItem(keyFor(baseUrl))
      }
    },
    [baseUrl],
  )

  return [model, setModel]
}
