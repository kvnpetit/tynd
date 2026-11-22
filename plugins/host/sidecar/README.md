# @tynd/plugin-sidecar

Sidecar facade for Tynd apps.

Declaring this plugin enables the host `sidecar` feature and exposes the typed
client from `@tynd/plugin-sidecar/client`.

## Install

```bash
tynd add sidecar
```

```ts
import { sidecar } from "@tynd/plugin-sidecar/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-sidecar` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-sidecar`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/sidecar/
