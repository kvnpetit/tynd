import { useEffect, useState } from "react"

interface Props {
  port: number
  onCommit(port: number): void
  disabled?: boolean
}

/**
 * Numeric input that commits on blur or Enter. Kept as a separate component
 * so the header doesn't have to manage the "draft vs committed" split itself.
 */
export function PortInput({ port, onCommit, disabled }: Props) {
  const [draft, setDraft] = useState(String(port))

  // Keep the draft in sync when the committed value changes externally
  // (e.g. reset button, persisted value loaded).
  useEffect(() => {
    setDraft(String(port))
  }, [port])

  function commit() {
    const next = Number.parseInt(draft, 10)
    if (Number.isFinite(next) && next > 0 && next <= 65535 && next !== port) {
      onCommit(next)
    } else {
      // Invalid or unchanged — snap the draft back.
      setDraft(String(port))
    }
  }

  return (
    <label className="port-input" title="Local server port">
      <span>:</span>
      <input
        type="text"
        inputMode="numeric"
        pattern="[0-9]*"
        value={draft}
        onChange={(e) => setDraft(e.target.value.replace(/[^0-9]/g, ""))}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault()
            ;(e.currentTarget as HTMLInputElement).blur()
          }
        }}
        disabled={disabled}
        maxLength={5}
        aria-label="Local server port"
      />
    </label>
  )
}
