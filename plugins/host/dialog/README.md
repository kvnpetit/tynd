# @tynd/plugin-dialog

Native dialog facade for Tynd apps.

Declaring this plugin enables the host `dialog` feature and exposes the typed
client from `@tynd/plugin-dialog/client`.

## Install

```bash
tynd add dialog
```

```ts
import { dialog } from "@tynd/plugin-dialog/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-dialog` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-dialog`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/dialog/
