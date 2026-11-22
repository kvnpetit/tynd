# @tynd/plugin-http-server

Embedded HTTP + WebSocket server for Tynd apps. Runs in the Bun backend
through `Bun.serve`; no plugin-specific Rust crate or extra JavaScript runtime.

## Install

```sh
tynd add http-server
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-http-server"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

## Use

```ts
import { httpServer } from "@tynd/plugin-http-server"

const server = await httpServer.serve(
  { port: 8080 },
  {
    async fetch(req) {
      const url = new URL(req.url)
      if (url.pathname === "/ping") return new Response("pong")
      return new Response("not found", { status: 404 })
    },
    websocket: {
      open(ws)        { ws.send("hello") },
      message(ws, d)  { ws.send(`echo: ${d}`) },
      close(ws)       { console.log(`#${ws.id} closed`) },
    },
  },
)

console.log(`listening on http://${server.host}:${server.port}`)

// Later — releases the port.
await server.stop()
```

## API

### `httpServer.serve(opts, handlers) → Promise<RunningServer>`

`opts`:

| Option | Type | Default | Notes |
| ------ | ---- | ------- | ----- |
| `port` | `number` | `0` (OS-picked) | |
| `host` | `string` | `"127.0.0.1"` | `"0.0.0.0"` exposes on the LAN. Installing this plugin explicitly adds the `network` host feature. Loopback hosts (`localhost`, `127.x.x.x`, `::1`) are checked against the `network-loopback` capability; every other host against `network-public`. |

`handlers`:

```ts
interface ServeHandlers {
  fetch?(req: Request, info: { remote: string }): Response | Promise<Response>
  websocket?: {
    open?(ws: ServerWebSocket): void | Promise<void>
    message?(ws: ServerWebSocket, data: string | Uint8Array): void | Promise<void>
    close?(ws: ServerWebSocket): void | Promise<void>
  }
}
```

Returns:

```ts
interface RunningServer {
  readonly id: number
  readonly host: string
  readonly port: number
  stop(): Promise<void>
}
```

`ServerWebSocket`:

```ts
interface ServerWebSocket {
  readonly id: number
  readonly remote: string
  send(data: string | Uint8Array): Promise<void>
  close(): Promise<void>
}
```

### `httpServer.list() → Promise<number[]>`

Currently-running server ids.

### Cookies

Handlers run in the webview, where `Set-Cookie` is a forbidden response-header
name: a `Response` built there silently loses it. Send cookies with
`setCookieHeader`, which the plugin turns into real `Set-Cookie` headers:

```ts
import { setCookieHeader } from "@tynd/plugin-http-server"

new Response("ok", { headers: [setCookieHeader(["a=1; Path=/", "b=2; HttpOnly"])] })
```

A plain `Set-Cookie` still works where the runtime keeps it (for example
tests under Bun), but is not reliable in a webview. Where
`Headers.getSetCookie()` is missing (Safari < 17), a single combined value is
forwarded instead of failing the request.

## Limits

| Constraint | Value |
| ---------- | ----- |
| Max concurrent servers | 32 |
| Max in-flight requests per server | 4096 |
| Max open WebSocket connections per server | 4096 |
| Max request body size | 16 MiB |
| Response timeout (per request) | 60 s |

Invalid options throw. Runtime request/WebSocket limits return HTTP 503;
body overflow returns 413. These bounds prevent runaway userland code
from exhausting host memory.

## Notes

* No TLS option is exposed yet. For HTTPS run behind a reverse proxy.
* Paths are routed userland-side via
  `req.url`. This keeps the plugin's wire surface small; if you need
  declarative routing reach for `itty-router`, `hono`, or similar.
* Loopback (`127.0.0.1` / `::1` / `localhost`) and public binds use
  separate core validation routes. Public exposure remains an explicit
  `host` choice in application backend code.

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
