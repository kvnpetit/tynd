import { createBackend } from "@tynd/core/client"
import type * as backend from "../backend/main"

/** Typed backend proxy — shared across hooks so components don't create duplicates. */
export const api = createBackend<typeof backend>()

export type { ChatMessage, ChatOptions } from "../backend/main"

/** `http://localhost:<port>` helper — the only shape the chatbot cares about. */
export function baseUrlForPort(port: number | string): string {
  return `http://localhost:${port}`
}
