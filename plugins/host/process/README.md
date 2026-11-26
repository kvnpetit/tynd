# @tynd/plugin-process

Process facade for Tynd apps.

Declaring this plugin enables the host `process` feature and exposes the typed
client from `@tynd/plugin-process/client`.

## Install

```bash
tynd add process
```

```ts
import { process } from "@tynd/plugin-process/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-process` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-process`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/process/
