# @tynd/plugin-deep-link

Deep-link facade for Tynd apps.

Declaring this plugin enables the host `deep-link` feature and exposes the typed
client from `@tynd/plugin-deep-link/client`.

## Install

```bash
tynd add deep-link
```

```ts
import { deepLink } from "@tynd/plugin-deep-link/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-deep-link` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-deep-link`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/deep-link/
