# @tynd/plugin-keyring

OS credential storage facade for Tynd apps.

Declaring this plugin enables the host `keyring` feature and exposes the typed
client from `@tynd/plugin-keyring/client`.

## Install

```bash
tynd add keyring
```

```ts
import { keyring } from "@tynd/plugin-keyring/client";

await keyring.set(
  { service: "com.example.notes", account: "session" },
  token,
);

const saved = await keyring.get({
  service: "com.example.notes",
  account: "session",
});

await keyring.delete({
  service: "com.example.notes",
  account: "session",
});
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-keyring` feature. A host without
it rejects the calls with an error naming `@tynd/plugin-keyring`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/keyring/
