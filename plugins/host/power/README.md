# @tynd/plugin-power

Power facade for Tynd apps.

Declaring this plugin enables the host `power` feature and exposes the typed
client from `@tynd/plugin-power/client`.

## Install

```bash
tynd add power
```

```ts
import { power } from "@tynd/plugin-power/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-power` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-power`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/power/
