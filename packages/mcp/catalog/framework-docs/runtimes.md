---
title: Runtime
description: Bun is the single JavaScript runtime used by every Tynd app.
---

# Bun runtime

Tynd runs every TypeScript backend on Bun. There is no runtime selector in
`tynd.config.ts`:

```ts
export default {
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

Bun provides JIT execution, `Bun.*`, Node-compatible APIs, `Buffer`, `process`,
`node:*` imports, npm packages, and native addons.

## Local HTTP servers

Use [backend RPC](https://kvnpetit.github.io/tynd/docs/v1.0/guides/backend-rpc) for calls between your WebView
and the Bun backend. If another process needs an HTTP endpoint, start one in
the backend with [`Bun.serve`](https://bun.com/docs/runtime/http/server):

```ts filename="backend/main.ts"
import { app } from "@tynd/core";

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(request) {
    if (request.method === "GET" && new URL(request.url).pathname === "/health") {
      return new Response("ok");
    }
    return new Response("Not found", { status: 404 });
  },
});

app.onClose(() => { void server.stop(true); });
app.start();
```

Read `server.url` for the address assigned by the OS. `Bun.serve` runs in the
trusted backend. Enforce authorization there before adding routes that read or
modify app data, including on loopback.

## Native host

Every app runs on a native Rust host that Tynd compiles for it with Cargo from
the `@tynd/host-src` dev dependency. The host always contains the core (`acl`,
`log`, window, webview, IPC) plus exactly the APIs the project declares: those
of its `plugins`, any names listed in `apis`, and `updater` when an `updater`
block is configured. Declaring a standalone plugin with a native crate links that Rust crate too.
JavaScript always executes on Bun.

`tynd dev` compiles the host with the Cargo `dev` profile and `tynd build` with
`release`, from the host package generated in `.tynd/build`; Cargo output goes to
the [shared host build cache](https://kvnpetit.github.io/tynd/docs/v1.0/concepts/build-pipeline#shared-host-build-cache). The first build takes a few minutes; a dev
run whose inputs did not change skips Cargo. `tynd start` runs the release host,
so it needs a prior `tynd build`.

```ts
export default {
  backend: "backend/main.ts",
  frontendDir: "dist",
  apis: ["fs", "http"],
  plugins: ["@tynd/plugin-sql"],
}
```

Compiling the host requires Bun 1.3+ and Rust 1.95+; see
[Installation](https://kvnpetit.github.io/tynd/docs/v1.0/getting-started/installation). You write no Rust
application code.
