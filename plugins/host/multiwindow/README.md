# @tynd/plugin-multiwindow

Secondary-window and child-webview facade for Tynd apps.

Declaring this plugin enables the host `multiwindow` feature and exposes the typed
client from `@tynd/plugin-multiwindow/client`.

## Install

```bash
tynd add multiwindow
```

```ts
import { multiwindow } from "@tynd/plugin-multiwindow/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-multiwindow` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-multiwindow`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/multiwindow/
