# tynd-security-policy

Pure filesystem, HTTP, process, shell and environment policy engine for the Tynd
native host.

## What it is

`tynd-security-policy` decides whether a host API call is allowed. It has no
host dispatch, event emission or storage side effects: `tynd-host` owns those
and delegates every policy decision here, which keeps the rules unit-testable
and fuzzable.

## Main types

- `PolicyInput` / `ScopeInput` — the deserialized `security` policy the app
  configures (`fs`, `http`, `defaultDeny`, `shellExecAllowedCommands`,
  `processAllowedBinaries`, `allowedSensitiveCapabilities`).
- `Scope` — compiled allow/deny glob sets. Filesystem patterns are normalized
  (separators unified, lowercased on Windows); HTTP patterns lowercase the
  scheme and authority and drop the default port. Deny always wins. The allow
  and deny lists are fixed once the policy is built; there are no runtime
  grants.
- `SecurityPolicy` — built with `SecurityPolicy::from_input(...)`. Checks:
  `check_fs`, `check_fs_resolved`, `check_http`, `check_process`,
  `check_shell_exec`, `check_shell_exec_with_env`,
  `check_sensitive_capability`, and `check_fs_with_grants` /
  `check_http_with_grants` with runtime grants.
- `Grants` / `PathGrant` / `FsAccess` — runtime scope grants (paths with
  directory / recursive / write flags, `http(s)://` URL patterns). A grant
  never overrides a deny rule and a read-only grant never admits a write.
- Helpers: `canonicalize_fs`, `canonicalize_http_url`,
  `canonicalize_process_binary`, `expand_tilde`, `matches_pattern`,
  `validate_glob_pattern`, and `env_key_denied` (blocks loader and profiler
  variables such as `COR_PROFILER` in spawned processes).

Checks return `Result<(), String>`; the error string is the message surfaced to
the caller. `check_shell_exec` refuses every command until
`shellExecAllowedCommands` is set.

## How Tynd uses it

`tynd-host` wraps the policy in `src/security/policy.rs` and calls it from the fs,
HTTP, process and shell APIs.

See [Security](https://kvnpetit.github.io/tynd/docs/v1.0/concepts/security) and
the [`security` API](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/security).

## Validation

```bash
cargo test -p tynd-security-policy
```

## License

Apache-2.0
