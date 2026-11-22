---
title: tyndWebview
description: "The current WebView: label, parent window, cookies, popup and download policies, and capability identity."
---

# `tyndWebview`

```ts
import { tyndWebview } from "@tynd/core/client";
```

`tyndWebview` describes the WebView the code runs in. A native window can host
several WebViews; each has its own label, IPC route, browsing-data store, and
`capabilities.webviews` identity. Creating, closing and moving child webviews
needs the [`multiwindow`](https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/multiwindow) plugin
(`multiwindow.webview.*`), added with `tynd add multiwindow` and imported from
`@tynd/plugin-multiwindow/client`.

```ts
console.log(tyndWebview.label());       // "preview"
console.log(tyndWebview.windowLabel()); // "main"
```

## Navigation events

`tyndWindow.onNavigation` and `tyndWindow.onPageLoad` filter with the current
WebView label. Native window events continue to filter with the parent window
label.

## New windows (`window.open`, `target="_blank"`)

`newWindow` on the `window` option of `app.start(...)` (primary window), `multiwindow.create` and `multiwindow.webview.create` sets what happens to popup requests:

| Value | Behavior |
|---|---|
| `"allow"` (default) | The platform opens its default popup, only for URLs that pass the navigation policy. The popup is an unmanaged webview without the Tynd IPC bridge |
| `"external"` | `http`, `https` and `mailto` URLs open in the system browser (throttled to one every 300 ms); other schemes are dropped. Needs `@tynd/plugin-shell` in `plugins`, otherwise the request is dropped with a warning |
| `"deny"` | Every request is dropped |

`"external"` lets a page open links in the system browser without a user gesture and without the `shell:openExternal` permission check, so prefer `"deny"` for webviews that show untrusted content.

```ts
import { multiwindow } from "@tynd/plugin-multiwindow/client";

await multiwindow.webview.create({ label: "docs", url: "https://example.com", x: 0, y: 0, width: 800, height: 600, newWindow: "deny" });

const off = tyndWebview.onNewWindow(({ label, url, action }) => {
  console.log(label, "asked for", url, "->", action); // action: "deny" | "external" | "allow"
});
```

Every request emits `webview:new-window`, whatever the decision. The event carries the popup URL (which can hold an OAuth `?code=`), so it is delivered to the webviews that show app content, whether the request came from an app-local or a remote webview. A webview that shows remote content never receives it.

## Downloads

Downloads are saved into the OS downloads directory (a numeric suffix avoids overwriting an existing file, including between concurrent downloads); there is no save dialog. Bidirectional-control and other invisible Unicode characters are stripped from the file name.

`downloads` on the `window` option of `app.start(...)`, `multiwindow.create` and `multiwindow.webview.create` sets the policy:

| Value | Behavior |
|---|---|
| unset (default) | Allowed only while the webview shows app content (bundled pages or the `tynd dev` server origin; any other loopback address is remote) and the download URL is itself app-local (or a `blob:` URL of app-local content); refused otherwise, so set `"allow"` to download remote files. `data:` downloads are refused too (they carry no origin, so a remote iframe inside local content could start one): set `"allow"` on that webview or window to permit them |
| `"allow"` | Always allowed, `data:` URLs included |
| `"deny"` | Every download is refused |

```ts
await multiwindow.webview.create({ label: "docs", url: "https://example.com", x: 0, y: 0, width: 800, height: 600, downloads: "allow" });
```

```ts
const off = tyndWebview.onDownload((e) => {
  if (e.type === "started") console.log(e.id, e.url, "->", e.path);
  else console.log(e.id, e.success ? "done" : "failed", e.path);
});
```

`started` carries `{ label, id, url, path }`; `completed` carries `{ label, id, url, path, success }` with the same `id`. Both events hold local file paths, so they are delivered like `webview:new-window`: to the webviews that show app content, never to a remote-content webview. On failure, and on macOS (WKWebView does not report it), the final `path` of `completed` is `null`. A download is refused if no downloads directory exists.

## Cookies

```ts
const cookies = await tyndWebview.cookies.get({ label: "preview", url: "https://example.com" });
await tyndWebview.cookies.set({ name: "sid", value: "abc", domain: "example.com", secure: true }, { label: "preview" });
await tyndWebview.cookies.delete({ name: "sid", domain: "example.com" }, { label: "preview" });
const removed = await tyndWebview.cookies.clear({ label: "preview" }); // number removed
```

`target` is `{ label?, url? }`: `label` defaults to the caller's webview and, when it names another webview, the caller must itself show app content (a webview that shows remote content can only access its own cookies), `url` restricts to cookies that apply to that `http(s)` URL (and supplies the domain for `set` when `domain` is omitted). A webview that shows remote content is further restricted to its own origin: `url` is required and its origin must match the webview's, a cookie `domain` must stay inside that host, and listing or clearing without `url` is refused. App-local callers are unchanged. Cookies include HttpOnly session tokens, so these four calls (`window:getWebviewCookies`, `window:setWebviewCookie`, `window:deleteWebviewCookie`, `window:clearWebviewCookies`) **always need an explicit capability grant**, even when no `window:` permission is declared. Not supported on Android.

## Proxy

```ts
app.start({ window: { proxy: "socks5://127.0.0.1:9050" } });                  // primary window
await multiwindow.webview.create({ label: "browser", url: "https://example.com", x: 0, y: 0, width: 800, height: 600,
  proxy: "socks5://127.0.0.1:9050" });                                       // child webview
```

Accepted forms are `http://host:port` (HTTP CONNECT) and `socks5://host:port`. No credentials and no path are allowed; `http` defaults to port 80, `socks5` needs an explicit port. An invalid value rejects the create call. Proxying works on every platform, including macOS 14+ (Tynd's minimum macOS).

## Permission prompts

Camera, microphone and geolocation requests from a page are not intercepted: the WebView applies the OS default behavior, and Tynd offers no handler to grant or deny them.

## Capability isolation

Child calls pass both identities to the capability authority:

```ts
app.start({
  capabilities: [{
    identifier: "preview-read",
    windows: ["main"],
    webviews: ["preview"],
    permissions: [{
      identifier: "preview-fs-read",
      commands: { allow: ["fs:readText", "fs:readBinary"] },
    }],
  }],
});
```

Both filters must match. Granting `windows: ["main"]` does not automatically
grant the command to every child WebView in that window when a `webviews`
filter is present.

## Related

- [Window API](https://tynd-apps.github.io/tynd/docs/v1.0/api/os/window)
- [`multiwindow` plugin](https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/multiwindow) — create and drive child webviews
- [Multi-Window and Multi-WebView guide](https://tynd-apps.github.io/tynd/docs/v1.0/guides/multi-window)
- [Security and capabilities](https://tynd-apps.github.io/tynd/docs/v1.0/concepts/security)
