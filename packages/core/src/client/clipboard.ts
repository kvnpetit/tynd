import { osCall } from "./_internal.js"

export interface ClipboardImage {
  /** Base64-encoded PNG bytes. */
  png: string
  width: number
  height: number
}

export const clipboard = {
  readText(): Promise<string> {
    return osCall("clipboard", "readText")
  },
  writeText(text: string): Promise<void> {
    return osCall("clipboard", "writeText", { text })
  },
  /**
   * Read HTML from the clipboard. arboard doesn't expose a reader — the OS
   * clipboard usually flattens HTML to text for other readers — so this
   * always returns `null`. Use `readText` as a fallback.
   */
  readHtml(): Promise<string | null> {
    return osCall("clipboard", "readHtml")
  },
  /**
   * Write rich HTML to the clipboard, with an optional plain-text fallback
   * for apps that don't understand `text/html`.
   */
  writeHtml(html: string, alt?: string): Promise<void> {
    return osCall("clipboard", "writeHtml", alt === undefined ? { html } : { html, alt })
  },
  /** Read a PNG image from the clipboard (null if the clipboard has no image). */
  readImage(): Promise<ClipboardImage | null> {
    return osCall("clipboard", "readImage")
  },
  /** Write a PNG image (base64-encoded bytes) to the clipboard. */
  writeImage(png: string): Promise<void> {
    return osCall("clipboard", "writeImage", { png })
  },
  /** Empty the clipboard. */
  clear(): Promise<void> {
    return osCall("clipboard", "clear")
  },

  /**
   * Start a polling thread that emits `clipboard:change` when the content
   * changes. Cross-OS — no native event API on Windows/macOS/Linux that
   * doesn't require heavy dependencies. Default interval 200 ms.
   */
  async onChange(
    handler: (event: { text: string }) => void,
    options?: { intervalMs?: number },
  ): Promise<() => Promise<void>> {
    await osCall("clipboard", "startMonitoring", {
      intervalMs: options?.intervalMs,
    })
    const off = window.__tynd__.os_on("clipboard:change", (raw) =>
      handler(raw as { text: string }),
    )
    return async () => {
      off()
      await osCall("clipboard", "stopMonitoring")
    }
  },
}
