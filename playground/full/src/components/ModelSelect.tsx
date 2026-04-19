interface Props {
  /** null while still probing the server. */
  models: string[] | null
  /** Currently-selected model id. Empty while nothing usable is loaded. */
  value: string
  onChange(model: string): void
  /** Disabled during streaming so the user can't switch model mid-reply. */
  disabled?: boolean
  /** Error message from the last fetch, if any. Shown as the placeholder. */
  error?: string | null
}

export function ModelSelect({ models, value, onChange, disabled, error }: Props) {
  const status: "loading" | "empty" | "ready" =
    models === null ? "loading" : models.length === 0 ? "empty" : "ready"

  // Keep a tiny footprint in the header — a native <select> is keyboard-
  // accessible, portal-free, and matches the OS look of the WebView.
  return (
    <select
      className="model-select"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      disabled={disabled || status !== "ready"}
      title={status === "ready" ? value : (error ?? "Loading models…")}
    >
      {status === "loading" && <option value="">Loading…</option>}
      {status === "empty" && <option value="">{error ? "Server error" : "No model"}</option>}
      {status === "ready" &&
        (models ?? []).map((m) => (
          <option key={m} value={m}>
            {m}
          </option>
        ))}
    </select>
  )
}
