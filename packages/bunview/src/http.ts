import fs from "fs";
import path from "path";

export interface DownloadProgress {
  /** Bytes received so far. */
  loaded: number;
  /** Total expected bytes from `Content-Length`, or `null` if unknown. */
  total: number | null;
  /** Percent 0–100, or `null` if total is unknown. */
  percent: number | null;
}

export interface DownloadOptions {
  /** Destination file path. Parent dirs are auto-created. */
  dest: string;
  /** Extra fetch headers (auth, User-Agent, etc.). */
  headers?: Record<string, string>;
  /** Called on every chunk. */
  onProgress?: (p: DownloadProgress) => void;
  /** Abort signal to cancel mid-download. */
  signal?: AbortSignal;
}

/**
 * Download a URL to disk with streaming progress callbacks.
 * Atomic: writes to `dest.download` first, then renames on success.
 */
export async function downloadFile(url: string, opts: DownloadOptions): Promise<void> {
  const response = await fetch(url, { headers: opts.headers, signal: opts.signal });
  if (!response.ok) {
    throw new Error(`[bunview] download failed: ${response.status} ${response.statusText}`);
  }

  const total = response.headers.get("content-length");
  const totalBytes = total ? parseInt(total, 10) : null;

  fs.mkdirSync(path.dirname(opts.dest), { recursive: true });
  const tmp = `${opts.dest}.download`;
  const out = fs.createWriteStream(tmp);

  let loaded = 0;
  const reader = response.body?.getReader();
  if (!reader) {
    out.close();
    throw new Error("[bunview] response has no body");
  }

  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      loaded += value.byteLength;
      out.write(value);
      opts.onProgress?.({
        loaded,
        total: totalBytes,
        percent: totalBytes ? Math.round((loaded / totalBytes) * 100) : null,
      });
    }
  } finally {
    out.end();
    await new Promise<void>((r) => out.once("close", () => r()));
  }

  fs.renameSync(tmp, opts.dest);
}
