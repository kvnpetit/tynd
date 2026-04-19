import { useCallback, useState } from "react"

const STORAGE_KEY = "tynd-chat:port"
const DEFAULT_PORT = 13305

function readInitial(): number {
  if (typeof window === "undefined") return DEFAULT_PORT
  const raw = window.localStorage.getItem(STORAGE_KEY)
  const parsed = raw ? Number.parseInt(raw, 10) : Number.NaN
  return Number.isFinite(parsed) && parsed > 0 && parsed <= 65535 ? parsed : DEFAULT_PORT
}

/**
 * Port state backed by localStorage so the user's chosen server survives
 * reloads. Falls back to {@link DEFAULT_PORT} on an empty or invalid value.
 */
export function usePersistedPort(): [number, (port: number) => void] {
  const [port, setPortState] = useState<number>(readInitial)

  const setPort = useCallback((next: number) => {
    setPortState(next)
    if (typeof window !== "undefined") {
      window.localStorage.setItem(STORAGE_KEY, String(next))
    }
  }, [])

  return [port, setPort]
}
