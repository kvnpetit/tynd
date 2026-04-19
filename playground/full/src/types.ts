export interface UiMessage {
  role: "user" | "assistant"
  content: string
  /** True while the assistant reply is still streaming in. */
  streaming?: boolean
}

export type ServerStatus = "probing" | "online" | "offline"
