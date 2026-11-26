# @tynd/plugin-window-state

Window-state facade for Tynd apps.

Declaring this plugin enables the host `window-state` feature and exposes the typed
client from `@tynd/plugin-window-state/client`.

## Install

```bash
tynd add window-state
```

```ts
import { windowState } from "@tynd/plugin-window-state/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-window-state` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-window-state`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/window-state/
