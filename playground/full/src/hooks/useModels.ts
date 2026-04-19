import { useEffect, useState } from "react"
import { api } from "../api"

export interface ModelsState {
  /** null = still probing. [] = loaded but no models reported. */
  models: string[] | null
  error: string | null
}

/**
 * Fetch the currently-loaded models from the server. Resets to "probing" on
 * every baseUrl change so the UI never displays stale entries from a
 * different server.
 */
export function useModels(baseUrl: string): ModelsState {
  const [state, setState] = useState<ModelsState>({ models: null, error: null })

  useEffect(() => {
    let cancelled = false
    setState({ models: null, error: null })
    api.listModels(baseUrl).then((r) => {
      if (cancelled) return
      if (r.ok) setState({ models: r.models, error: null })
      else setState({ models: [], error: r.error })
    })
    return () => {
      cancelled = true
    }
  }, [baseUrl])

  return state
}
