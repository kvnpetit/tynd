# @tynd/plugin-oauth

Localhost OAuth callback receiver for Tynd apps. Spawns a one-shot HTTP
listener on `127.0.0.1`, captures the redirect query, returns the parsed
params. Listener runs in the app's Bun backend and self-tears-down after
the redirect arrives.

## Install

```sh
tynd add oauth
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-oauth"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

## Use

```ts
import { oauth } from "@tynd/plugin-oauth"
import { shell } from "@tynd/plugin-shell/client"

const state = crypto.randomUUID()

// 1. Spawn a listener on a free port and get the redirect_uri back.
const flow = await oauth.start({ path: "/callback", expectedState: state })

// 2. Open the provider's authorize page with the flow's redirect_uri.
await shell.open(
  `https://github.com/login/oauth/authorize?` +
  `client_id=YOUR_CLIENT_ID&` +
  `redirect_uri=${encodeURIComponent(flow.redirectUri)}&` +
  `state=${encodeURIComponent(state)}&` +
  `scope=user:email`,
)

// 3. Wait for the user to complete the flow in their browser.
//    Resolves with the parsed query params from the redirect.
const { params } = await flow.wait({ timeoutMs: 120_000 })
console.log("Auth code:", params.code)
```

The browser sees a polite "you can close this tab" page and the listener
shuts down.

## API

### `oauth.start(opts?) → Promise<OAuthFlow>`

| Option | Type | Default | Notes |
| ------ | ---- | ------- | ----- |
| `port` | `number` | `0` (OS-picked) | Pin a port if your provider requires a fixed `redirect_uri`. |
| `path` | `string` | `"/"` | Path the provider redirects to. |
| `successHtml` | `string` | built-in | HTML returned to the browser after the callback fires. |
| `expectedState` | `string` | — | Printable-ASCII CSRF nonce sent to the authorize endpoint. When set, callbacks with a missing, duplicated, or different query `state` are rejected with HTTP 400 and surface as `oauth:error` instead of `oauth:callback`. **Always pass this** for any flow that uses `state` (per RFC 6749 §10.12). |

Returns:

```ts
interface OAuthFlow {
  id: number
  port: number
  redirectUri: string
  wait(opts?: { timeoutMs?: number }): Promise<{ params: Record<string, string> }>
  cancel(): Promise<void>
}
```

### `oauth.list() → Promise<number[]>`

Currently-active flow ids — useful for debug overlays.

## Security

> **`successHtml` is rendered raw.** The string is shipped to the
> browser as-is with `Content-Type: text/html`. The default value is
> safe; **never** interpolate provider responses, query parameters, or
> any other untrusted data into a custom value, or you open an XSS
> vector on the loopback origin (which can read other localhost
> services). Treat `successHtml` as caller-trusted markup only — if you
> need to display dynamic data on the success page, build a static
> shell that fetches from your backend after the callback fires.
>
> Defence-in-depth on top of caller discipline:
>
> * **Length cap** — `successHtml` is rejected at `start()` if it is
>   larger than 4 KiB.
> * **ASCII-only** — non-ASCII bytes (and NUL) are rejected at
>   `start()`. Encode characters outside the ASCII range as HTML
>   entities.
> * **`Content-Security-Policy`** — the response carries
>   `default-src 'none'; style-src 'unsafe-inline'; script-src 'none'; frame-ancestors 'none'`,
>   so even a malformed value cannot execute scripts or load remote
>   resources.
> * **`path`** — the redirect path is restricted to a single leading
>   `/`, ASCII-only, capped at 256 bytes, and must not contain `?` or
>   `#`.

## Notes

* Loopback only (`127.0.0.1`). Public-facing OAuth flows belong on a
  hosted callback URL, not a desktop app.
* Only `GET` can complete a callback. Other methods return HTTP 405 and
  leave the flow active.
* One callback per `start()`. The listener self-tears-down after
  responding so a single `OAuthFlow` can't outlive its redirect.
* `wait()` defaults to a 5-minute timeout; pass `timeoutMs` to override.
* If the user cancels in the browser, call `flow.cancel()` to drop
  the listener early — otherwise it sits idle until `timeoutMs`.

---

See also: [Standalone plugin index](../README.md).
