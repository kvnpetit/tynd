---
title: Multi-Window and Multi-WebView
description: "Create native windows or compose multiple WebViews inside one window, with isolated routing and lifecycle."
---

# Multi-Window and Multi-WebView

The primary window has label `"main"`. Additional windows are created with a unique label and get their own WebView + IPC channel.

Creating windows and child WebViews needs the `multiwindow` official plugin:

```bash
tynd add multiwindow
```

## Create a secondary window

```ts
import { multiwindow } from "@tynd/plugin-multiwindow/client";

await multiwindow.create({
  label: "settings",
  url: "/settings",         // optional — defaults to the primary entry
  title: "Settings",
  width: 600,
  height: 480,
});
```

`tyndWindow` and `tyndWebview` from `@tynd/core/client` **auto-target the window and WebView they run in** — no label argument needed.

## Compose WebViews inside one window

Use a child WebView when regions need independent documents, origins, storage,
navigation, or capability identities without creating another OS window:

```ts
import { multiwindow } from "@tynd/plugin-multiwindow/client";

await multiwindow.webview.create({
  label: "preview",
  x: 360,
  y: 0,
  width: 920,
  height: 720,
  url: "/preview",
  incognito: true,
});

await multiwindow.webview.setBounds("preview", {
  x: 400,
  y: 64,
  width: 880,
  height: 656,
});
```

Inside the child, `tyndWebview.label()` returns `"preview"` while
`tyndWebview.windowLabel()` and `tyndWindow.label()` return `"main"`.
Closing the parent closes every child and cancels their streams.

On Linux, root and child WebViews use the same GTK composition path. Native
composition works on X11, XWayland, and pure Wayland. Maintainers validate X11
and pure Wayland through explicit local/manual release runs; automated CI never
starts a compositor or WebView.

## Enumerate + close

```ts
const labels = await multiwindow.all();   // ["main", "settings", ...]
await multiwindow.close("settings");       // "main" cannot be closed this way
await multiwindow.closeSelf();             // from inside a secondary window

console.log(tyndWindow.label());          // "main" or "settings" depending on caller
```

## Event routing

Window events (`onResized`, `onMoved`, `onFocused`, …) are delivered to **every app-local webview** (never to remote content) with a `label` field in the payload. The client filters them by `__TYND_WINDOW_LABEL__`, so child WebViews observe their parent window. Navigation and page-load hooks filter by the separate `__TYND_WEBVIEW_LABEL__`.

```ts
// Inside the settings window — handler only fires for "settings".
tyndWindow.onResized(({ width, height }) => {
  console.log("settings window resized to", width, height);
});
```

<Callout type="info">
  Don't try to "listen for the main window's events from the settings window" — the routing logic filters that out. Coordinate cross-window state via the backend instead.
</Callout>

## Cross-window coordination

Two patterns:

### Via the backend (recommended)

The backend is a single JS context shared by every window. Use an emitter as the event bus:

```ts filename="backend/main.ts"
export const events = createEmitter<{
  themeChanged: { theme: "light" | "dark" };
}>();

export async function setTheme(theme: "light" | "dark") {
  events.emit("themeChanged", { theme });
  // persist via store, etc.
}
```

```ts
// any window
api.on("themeChanged", ({ theme }) => applyTheme(theme));
api.setTheme("dark");
```

`emit` broadcasts to every subscribed app-local webview (never to remote-content webviews). Combined with per-window event filtering, each window gets the update.

### Via `store` + manual refresh

For state that doesn't change often, write to `store` from the producer and re-read from the consumer on explicit refresh triggers.

## Per-window routes

Frontend routing (React Router, Vue Router, Svelte kit-free router, …) decides what's rendered inside each window. Pass the desired route as `url`:

```ts
await multiwindow.create({ label: "about", url: "/about", title: "About" });
```

Inside the `about` window your router sees `window.location.pathname === "/about"` and renders the corresponding view.

## Per-window RPC + streaming

Every window uses the same `createBackend<typeof backend>()` import — each call / stream is independent. Streams originating in a window are automatically canceled when that window closes (`cancel_streams_for_label` in the Rust host).

## Window lifecycle

```ts
// in any window
tyndWindow.onCloseRequested((e) => {
  if (hasUnsavedChanges()) {
    e.preventDefault();
    void showSavePrompt();
  }
});

// manual cancel e.g. from a modal opened elsewhere during the 500ms window:
await tyndWindow.cancelClose();
```

- `onCloseRequested` lets you preventDefault() synchronously to cancel the close.
- If nothing cancels within 500 ms, the close proceeds.
- If you `preventDefault`, close the window yourself when ready: `multiwindow.closeSelf()` in a secondary window, `app.exit()` in the primary one.

## Sizing, positioning, monitors

Each window has its own geometry:

```ts
await tyndWindow.setSize(1400, 900);
await tyndWindow.setPosition(100, 100);
const pos = await tyndWindow.getPosition();
const size = await tyndWindow.getSize();
const dpi = await tyndWindow.scaleFactor();   // 1.0 / 1.5 / 2.0
```

Enumerate monitors:

```ts
import { monitors } from "@tynd/core/client";

const all = await monitors.all();
const primary = await monitors.primary();
const current = await monitors.current();   // where the calling window is
```

## Gotchas

- **`multiwindow.close("main")`** is rejected and `multiwindow.closeSelf()` is a no-op in the primary window. To quit the app, call `app.exit()` or let the user click the OS close button.
- **Window events are broadcast**, not routed. Always rely on `tyndWindow.on*` helpers, not manually subscribing to raw `window:*` events, so the label filtering does its job.
- **Streams don't survive window close.** If a user closes Settings mid-stream, the generator on the backend is canceled.

## Next

- [Window API reference](https://tynd-apps.github.io/tynd/docs/v1.0/api/os/window)
- [WebView API reference](https://tynd-apps.github.io/tynd/docs/v1.0/api/os/webview)
- [`multiwindow` plugin reference](https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/multiwindow)
- [Menu API](https://tynd-apps.github.io/tynd/docs/v1.0/api/os/menu)
- [Monitor API](https://tynd-apps.github.io/tynd/docs/v1.0/api/os/monitor)
