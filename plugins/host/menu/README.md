# @tynd/plugin-menu

Application and context menu facade for Tynd apps.

Declaring this plugin enables the host `menu` feature and exposes the typed
client from `@tynd/plugin-menu/client`.

## Install

```bash
tynd add menu
```

```ts
import { menu } from "@tynd/plugin-menu/client";
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`.
The app's host is then compiled with the `api-menu` feature. A host without
it rejects the calls with an error naming
`@tynd/plugin-menu`.

The menu item types (`MenuItem`, `MenuSubmenu`, ...) are exported from the
package root: `import type { MenuItem } from "@tynd/plugin-menu"`.

Docs: https://kvnpetit.github.io/tynd/docs/v1.0/api/plugins/menu/
