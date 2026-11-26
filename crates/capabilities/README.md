# tynd-capabilities

Pure capability, ACL and identifier contracts for the Tynd native host.

## What it is

`tynd-capabilities` owns the data shapes and deterministic matching helpers of
Tynd's capability system. It has no runtime state, disk persistence, IPC
dispatch or OS integration: those stay in `tynd-host`, which calls this crate to
decide whether a command is allowed.

## Main types

- `Capability` — one grant: `identifier`, `windows`, `webviews`, `local`,
  optional `remote` URLs (`RemoteCapability`), `permissions` and `platforms`.
- `CapabilityFile` — accepted file shapes (single capability, list, or
  `{ capabilities: [...] }`), parsed with `CapabilityFile::from_bytes`.
- `Manifest` / `Permission` / `Commands` — a plugin's permission manifest: named
  permissions with `allow` / `deny` command lists, permission sets and scopes.
- `Resolved` — the result of `Resolved::resolve(manifests, capabilities, target)`;
  `is_allowed(...)` answers per command, window, webview and origin.
- `Target` — platform filter (`Target::current()` for the running OS).
- Identifier newtypes: `ApiName`, `PluginName`, `MethodName`, `WindowLabel`,
  `CapabilityId`, `EventName`, `Command`.
- Helpers: `plugin_command_key`, `manifest_command_key`,
  `validate_capability_shape`, `capability_schema` (JSON schema of a capability).

## How Tynd uses it

`tynd-host` resolves the app's `capabilities` at startup (`security/acl.rs`) and checks
each IPC call against the `Resolved` set in `app/dispatch.rs`. Command keys for
native plugins are built with `plugin_command_key(plugin, method)`.

See [Security](https://kvnpetit.github.io/tynd/docs/v1.0/concepts/security)
for the user-facing model.

## Validation

```bash
cargo test -p tynd-capabilities
```

## License

Apache-2.0
