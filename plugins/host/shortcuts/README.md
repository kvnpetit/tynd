# @tynd/plugin-shortcuts

Shortcuts facade for Tynd apps.

Declaring this plugin enables the host `shortcuts` feature and exposes the typed
client from `@tynd/plugin-shortcuts/client`.

## Install

```bash
tynd add shortcuts
```

```ts
import { shortcuts } from "@tynd/plugin-shortcuts/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-shortcuts` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-shortcuts`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/shortcuts/
