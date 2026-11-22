# @tynd/plugin-http

Host HTTP facade for Tynd apps.

Declaring this plugin enables the host `http` feature and exposes the typed
client from `@tynd/plugin-http/client`.

## Install

```bash
tynd add http
```

```ts
import { http } from "@tynd/plugin-http/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-http` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-http`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/http/
