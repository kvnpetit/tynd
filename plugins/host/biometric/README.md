# @tynd/plugin-biometric

Biometric authentication facade for Tynd apps.

Declaring this plugin enables the host `biometric` feature and exposes the typed
client from `@tynd/plugin-biometric/client`.

## Install

```bash
tynd add biometric
```

```ts
import { biometric } from "@tynd/plugin-biometric/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-biometric` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-biometric`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/biometric/
