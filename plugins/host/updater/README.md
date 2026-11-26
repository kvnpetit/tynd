# @tynd/plugin-updater

Updater facade for Tynd apps.

Declaring this plugin enables the host `updater` feature and exposes the typed
client from `@tynd/plugin-updater/client`.

## Install

```bash
tynd add updater
```

```ts
import { updater } from "@tynd/plugin-updater/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-updater` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-updater`.

`updater.install` swaps and relaunches the binary on Windows and Linux, and
replaces the `.app` bundle from an `app-archive` update on macOS.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/updater/
