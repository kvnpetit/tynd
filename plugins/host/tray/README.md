# @tynd/plugin-tray

System-tray facade for Tynd apps.

Declaring this plugin enables the host `tray` feature and exposes the typed
client from `@tynd/plugin-tray/client`.

## Install

```bash
tynd add tray
```

```ts
import { tray } from "@tynd/plugin-tray/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-tray` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-tray`.

The `TrayConfig` type is exported from the package root:
`import type { TrayConfig } from "@tynd/plugin-tray"`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/tray/
