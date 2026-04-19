import { useEffect, useState } from "react"
import { api } from "../api"
import type { ServerStatus } from "../types"

/**
 * Probe the configured server whenever the URL changes. Resets to "probing"
 * immediately so the UI reflects the intent even if the fetch is slow.
 */
export function useServerStatus(baseUrl: string): ServerStatus {
  const [status, setStatus] = useState<ServerStatus>("probing")

  useEffect(() => {
    let cancelled = false
    setStatus("probing")
    api.pingServer(baseUrl).then((r) => {
      if (!cancelled) setStatus(r.ok ? "online" : "offline")
    })
    return () => {
      cancelled = true
    }
  }, [baseUrl])

  return status
}
