# Host-backed official plugins

Host-backed official plugins are tiny package facades for optional native host
APIs.

They live here instead of `packages/*` so the framework core stays visually and
architecturally separate from optional native capabilities.

Each package keeps its public npm name, for example:

```json
{
  "name": "@tynd/plugin-fs",
  "tynd": {
    "apis": ["fs"],
    "kind": "host"
  }
}
```

When an app declares a host-backed plugin, the CLI reads `tynd.apis` and compiles the
matching Cargo `api-*` feature into the app's host.
