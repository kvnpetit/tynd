# @tynd/plugin-compute

Compute (hashing and random bytes) facade for Tynd apps.

Declaring this plugin enables the host `compute` feature and exposes the typed
client from `@tynd/plugin-compute/client`.

## Install

```bash
tynd add compute
```

```ts
import { compute } from "@tynd/plugin-compute/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-compute` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-compute`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/compute/
