# @tynd/plugin-system-stats

System statistics facade for Tynd apps.

Declaring this plugin enables the host `system-stats` feature and exposes the typed
client from `@tynd/plugin-system-stats/client`.

## Install

```bash
tynd add system-stats
```

```ts
import { systemStats } from "@tynd/plugin-system-stats/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-system-stats` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-system-stats`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/system-stats/
