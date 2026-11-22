# tynd-update-core

Pure updater contract helpers shared by the Tynd host and the bundler.

## What it is

`tynd-update-core` holds the side-effect-free logic of Tynd auto-updates:
version comparison, the updater config shape, endpoint templating, mirror
fallback, key rotation and staged rollout. It does no networking, disk I/O or
signature verification itself; callers pass those in as closures. One tested
implementation is shared instead of being duplicated in each binary.

## Main API

- `UpdaterConfig` — the `updater` config block (`endpoints`, `publicKeys`,
  `channel`, patch, check and download options), mirroring the CLI config
  schema. `resolved_endpoints()` / `resolved_public_keys()` return the lists
  trimmed and de-duplicated. `PromptStyle` selects how a pending update is shown.
- Versions: `is_update_available(current, latest)` (strict SemVer),
  `meets_requirement(version, req)`, `version_eq`.
- `hashes_match` — constant-time comparison of hex digests.
- Channels and endpoints: `DEFAULT_CHANNEL` (`stable`), `validate_channel`,
  `render_endpoint` with `TemplateVars` (replaces `{{channel}}`, `{{target}}`,
  `{{arch}}`, `{{current_version}}`, percent-encoded).
- Mirrors: `try_endpoints` tries each endpoint in order; a
  `FetchFailure::Retryable` (network error, `is_retryable_status`) moves on, a
  `FetchFailure::Fatal` stops.
- Key rotation: `split_key_list`, `first_verifying_key` (a signature is valid
  when any trusted key verifies it).
- Staged rollout: `rollout_bucket` (stable per install and release) and
  `rollout_allows(rollout, bucket)`; a missing `rollout` means everyone.
- `PatchCompression` — `Zstd` or `None` for binary patches.

## How Tynd uses it

- `tynd-host` (`src/api/updater`) checks, downloads and verifies updates with
  these helpers.
- `tynd-bundler` (`crates/bundler`) uses `PatchCompression` when it
  writes patches.

See [Auto-updates](https://tynd-apps.github.io/tynd/docs/v1.0/guides/auto-updates).

## Validation

```bash
cargo test -p tynd-update-core
```

## License

Apache-2.0
