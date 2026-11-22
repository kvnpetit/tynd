# @tynd/plugin-store

Key-value store facade for Tynd apps.

Declaring this plugin enables the host `store` feature and exposes the typed
client from `@tynd/plugin-store/client`.

## Install

```bash
tynd add store
```

```ts
import { createStore } from "@tynd/plugin-store/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-store` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-store`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/store/
