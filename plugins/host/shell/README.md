# @tynd/plugin-shell

Shell (open URLs and paths) facade for Tynd apps.

Declaring this plugin enables the host `shell` feature and exposes the typed
client from `@tynd/plugin-shell/client`.

## Install

```bash
tynd add shell
```

```ts
import { shell } from "@tynd/plugin-shell/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-shell` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-shell`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/shell/
