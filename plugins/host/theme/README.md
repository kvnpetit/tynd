# @tynd/plugin-theme

Theme facade for Tynd apps.

Declaring this plugin enables the host `theme` feature and exposes the typed
client from `@tynd/plugin-theme/client`.

## Install

```bash
tynd add theme
```

```ts
import { theme } from "@tynd/plugin-theme/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-theme` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-theme`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/theme/
