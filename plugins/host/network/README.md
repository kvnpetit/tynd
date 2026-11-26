# @tynd/plugin-network

Network facade for Tynd apps.

Declaring this plugin enables the host `network` feature and exposes the typed
client from `@tynd/plugin-network/client`.

## Install

```bash
tynd add network
```

```ts
import { network } from "@tynd/plugin-network/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-network` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-network`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/network/
