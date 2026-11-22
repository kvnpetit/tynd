# @tynd/plugin-websocket

WebSocket facade for Tynd apps.

Declaring this plugin enables the host `websocket` feature and exposes the typed
client from `@tynd/plugin-websocket/client`.

## Install

```bash
tynd add websocket
```

```ts
import { websocket } from "@tynd/plugin-websocket/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-websocket` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-websocket`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/websocket/
