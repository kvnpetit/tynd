# @tynd/plugin-single-instance

Single-instance facade for Tynd apps.

Declaring this plugin enables the host `single-instance` feature and exposes the typed
client from `@tynd/plugin-single-instance/client`.

## Install

```bash
tynd add single-instance
```

```ts
import { singleInstance } from "@tynd/plugin-single-instance/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-single-instance` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-single-instance`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/single-instance/
