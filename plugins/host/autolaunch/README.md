# @tynd/plugin-autolaunch

Autolaunch facade for Tynd apps.

Declaring this plugin enables the host `autolaunch` feature and exposes the typed
client from `@tynd/plugin-autolaunch/client`.

## Install

```bash
tynd add autolaunch
```

```ts
import { autolaunch } from "@tynd/plugin-autolaunch/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-autolaunch` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-autolaunch`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/autolaunch/
