import type { UiMessage } from "../types"

export function Message({ role, content, streaming }: UiMessage) {
  return (
    <div className={`msg msg-${role}`}>
      <div className="msg-role">{role === "user" ? "You" : "Assistant"}</div>
      <div className="msg-body">
        {content}
        {streaming && <span className="caret">▍</span>}
      </div>
    </div>
  )
}
