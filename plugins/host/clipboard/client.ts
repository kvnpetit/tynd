import { osCall, osOn } from "@tynd/core/client/bridge";

/** Image returned by `clipboard.readImage`. */
export interface ClipboardImage {
  /** Base64-encoded PNG bytes. */
  png: string;
  width: number;
  height: number;
}

// Number of live `onChange` subscriptions in this webview. The host watcher is
// a process-wide singleton, so it is started by the first subscriber and
// stopped only when the last one leaves.
let monitorSubscribers = 0;
let monitorStart: Promise<unknown> | null = null;

/** Frontend bindings for the host's `clipboard` API (text/HTML/image plus polling). */
export const clipboard = {
  /** Read the system clipboard's text content, empty string if none. */
  readText(): Promise<string> {
    return osCall("clipboard", "readText");
  },
  /** Replace the system clipboard with the given text. */
  writeText(text: string): Promise<void> {
    return osCall("clipboard", "writeText", { text });
  },
  /**
   * Read HTML from the clipboard, or `null` when no HTML representation exists.
   */
  readHtml(): Promise<string | null> {
    return osCall("clipboard", "readHtml");
  },
  /**
   * Write rich HTML to the clipboard, with an optional plain-text fallback
   * for apps that don't understand `text/html`.
   */
  writeHtml(html: string, alt?: string): Promise<void> {
    return osCall("clipboard", "writeHtml", alt === undefined ? { html } : { html, alt });
  },
  /** Read file references from the clipboard, or `[]` when none are present. */
  readFiles(): Promise<string[]> {
    return osCall("clipboard", "readFiles");
  },
  /** Replace the clipboard with one or more file references. */
  writeFiles(paths: string[]): Promise<void> {
    return osCall("clipboard", "writeFiles", { paths });
  },
  /** Read a PNG image from the clipboard (null if the clipboard has no image). */
  readImage(): Promise<ClipboardImage | null> {
    return osCall("clipboard", "readImage");
  },
  /** Write a PNG image (base64-encoded bytes) to the clipboard. */
  writeImage(png: string): Promise<void> {
    return osCall("clipboard", "writeImage", { png });
  },
  /** Empty the clipboard. */
  clear(): Promise<void> {
    return osCall("clipboard", "clear");
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
    monitorSubscribers++;
    if (monitorSubscribers === 1) {
      monitorStart = osCall("clipboard", "startMonitoring", {
        intervalMs: options?.intervalMs,
      });
    }
    try {
      await monitorStart;
    } catch (e) {
      monitorSubscribers--;
      if (monitorSubscribers === 0) monitorStart = null;
      throw e;
    }
    const off = osOn("clipboard:change", (raw) => handler(raw as { text: string }));
    let disposed = false;
    return async () => {
      if (disposed) return;
      disposed = true;
      off();
      monitorSubscribers--;
      if (monitorSubscribers === 0) {
        monitorStart = null;
        await osCall("clipboard", "stopMonitoring");
      }
    };
  },
};
