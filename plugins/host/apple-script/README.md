# @tynd/plugin-apple-script

AppleScript/JXA facade for Tynd apps.

Declaring this plugin enables the macOS-only host `apple-script` feature and
exposes the typed client from `@tynd/plugin-apple-script/client`.

## Install

```bash
tynd add apple-script
```

```ts
import { appleScript } from "@tynd/plugin-apple-script/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-apple-script` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-apple-script`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/apple-script/
