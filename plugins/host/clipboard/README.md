# @tynd/plugin-clipboard

Clipboard facade for Tynd apps.

Declaring this plugin enables the host `clipboard` feature and exposes the typed
client from `@tynd/plugin-clipboard/client`.

## Install

```bash
tynd add clipboard
```

```ts
import { clipboard } from "@tynd/plugin-clipboard/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-clipboard` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-clipboard`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/clipboard/
