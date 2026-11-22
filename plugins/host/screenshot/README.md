# @tynd/plugin-screenshot

Screenshot facade for Tynd apps.

Declaring this plugin enables the host `screenshot` feature and exposes the typed
client from `@tynd/plugin-screenshot/client`.

## Install

```bash
tynd add screenshot
```

```ts
import { screenshot } from "@tynd/plugin-screenshot/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-screenshot` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-screenshot`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/screenshot/
