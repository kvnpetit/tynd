/**
 * Bunview Asset Server (fallback / dev / embedded mode)
 *
 * Used only when zero-network file serving is unavailable:
 *   1. Embedded mode (prod) — serves from $bunfs virtual filesystem
 *   2. Dev mode (HMR)       — WebSocket and static assets not needed here
 *                             (Vite serves files; this server is skipped)
 *
 * Events are now delivered via webview_eval (C++ → JS) and webview_bind
 * (JS → C++) — no WebSocket, no TCP port involved.
 */

import path from "path";

export class BunviewServer {
  private bunServer!: ReturnType<typeof Bun.serve>;

  constructor(
    private readonly staticDir: string,
    private readonly port = 0,
  ) {}

  async start(): Promise<number> {
    const staticDir = this.staticDir;

    this.bunServer = Bun.serve({
      port: this.port,
      hostname: "127.0.0.1",

      fetch(req) {
        const url = new URL(req.url);
        return serveStatic(url.pathname, staticDir);
      },
    });

    return this.bunServer.port as number;
  }

  stop() {
    this.bunServer?.stop(true);
  }

  get url(): string {
    return `http://localhost:${this.bunServer.port}`;
  }
}

/** Embedded files map — populated at build time by the generated entry wrapper. */
const embeddedFiles: Record<string, string> | null =
  (globalThis as any).__BUNVIEW_EMBEDDED__?.files ?? null;

async function serveStatic(pathname: string, staticDir: string): Promise<Response> {
  const normalized = pathname === "/" ? "/index.html" : pathname;
  if (embeddedFiles) {
    const filePath = embeddedFiles[normalized];
    if (filePath) {
      const file = Bun.file(filePath);
      if (normalized.endsWith(".html")) return injectAndServe(file);
      const ct = mimeType(normalized);
      return ct ? new Response(file, { headers: { "Content-Type": ct } }) : new Response(file);
    }
    const indexPath = embeddedFiles["/index.html"];
    if (indexPath) return injectAndServe(Bun.file(indexPath));
    return new Response("Not found", { status: 404 });
  }
  const filePath = path.join(staticDir, normalized);
  const file     = Bun.file(filePath);

  if (!(await file.exists())) {
    const index = Bun.file(path.join(staticDir, "index.html"));
    if (await index.exists()) return injectAndServe(index);
    return new Response("Not found", { status: 404 });
  }

  return filePath.endsWith(".html") ? injectAndServe(file) : new Response(file);
}

async function injectAndServe(file: ReturnType<typeof Bun.file>): Promise<Response> {
  let html = await file.text();
  const tag = `<script src="/__bunview__/client.js"></script>`;
  html = html.includes("</head>")
    ? html.replace("</head>", tag + "\n</head>")
    : tag + "\n" + html;
  return new Response(html, { headers: { "Content-Type": "text/html; charset=utf-8" } });
}

const MIME: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".css":  "text/css; charset=utf-8",
  ".js":   "application/javascript; charset=utf-8",
  ".mjs":  "application/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg":  "image/svg+xml",
  ".png":  "image/png",
  ".jpg":  "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif":  "image/gif",
  ".webp": "image/webp",
  ".ico":  "image/x-icon",
  ".woff": "font/woff",
  ".woff2":"font/woff2",
  ".ttf":  "font/ttf",
  ".otf":  "font/otf",
  ".wasm": "application/wasm",
  ".mp3":  "audio/mpeg",
  ".mp4":  "video/mp4",
  ".webm": "video/webm",
};

function mimeType(pathname: string): string | null {
  const ext = pathname.slice(pathname.lastIndexOf("."));
  return MIME[ext] ?? null;
}
