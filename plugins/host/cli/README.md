# @tynd/plugin-cli

Command-line arguments facade for Tynd apps.

Declaring this plugin enables the host `cli` feature and exposes the typed
client from `@tynd/plugin-cli/client`.

## Install

```bash
tynd add cli
```

```ts
import { cli } from "@tynd/plugin-cli/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-cli` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-cli`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/cli/
