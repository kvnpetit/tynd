# @tynd/plugin-vault

Vault facade for Tynd apps.

Declaring this plugin enables the host `vault` feature and exposes the typed
client from `@tynd/plugin-vault/client`.

## Install

```bash
tynd add vault
```

```ts
import { vault } from "@tynd/plugin-vault/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-vault` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-vault`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/vault/
