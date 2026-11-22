# @tynd/plugin-notification

Notification facade for Tynd apps.

Declaring this plugin enables the host `notification` feature and exposes the typed
client from `@tynd/plugin-notification/client`.

## Install

```bash
tynd add notification
```

```ts
import { notification } from "@tynd/plugin-notification/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-notification` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-notification`.

Docs: https://tynd-apps.github.io/tynd/docs/v1.0/api/plugins/notification/
