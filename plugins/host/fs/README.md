# @tynd/plugin-fs

Filesystem facade for Tynd apps.

Declaring this plugin enables the host `fs` feature and exposes the typed
client from `@tynd/plugin-fs/client`.

## Install

```bash
tynd add fs
```

```ts
import { fs } from "@tynd/plugin-fs/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-fs` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-fs`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/fs/
