---
title: Auto-Updates
description: "Build, sign, publish, verify, and install Tynd updates with Ed25519 and optional native binary patches."
---

# Auto-Updates

Tynd updater trust has two signed layers:

1. `manifest.json.sig` authenticates the exact manifest bytes.
2. Each full artifact and optional patch carries its own Ed25519 signature. A patched binary is verified again with the full-release signature after reconstruction.

Windows/Linux update payloads are packed TYNDPKG executables. macOS uses a signed `.app` archive so the bundle structure and platform signature survive installation.

## One-time setup

### Generate an app key

```bash
tynd updater keygen --app-id com.example.myapp
```

Default private-key path:

```text
~/.tynd/keys/com.example.myapp.key
```

`updater keygen` uses native Rust OS randomness and `ed25519-dalek`. It prints the base64 public key and, unless `--no-config-edit` is present, adds an updater block/public key to a supported config file when safe.

Back up the private key and never commit it. Losing it prevents existing installs from trusting future releases.

### Configure the endpoint

```ts filename="tynd.config.ts"
import { defineConfig } from "@tynd/cli";

export default defineConfig({
  backend: "backend/main.ts",
  frontendDir: "dist",
  bundle: {
    identifier: "com.example.myapp",
  },
  updater: {
    endpoints: ["https://updates.example.com/manifest.json"],
    publicKeys: ["<base64 32-byte Ed25519 public key>"],
  },
});
```

The host only trusts the version and `updater` configuration that the backend sends at startup; renderer `updaterIpc` calls cannot choose the version, endpoints or keys (see [updater trust model](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/updater#trust-model)). Every endpoint must be HTTP(S). Every key must decode to exactly 32 bytes. Adding `updater` makes the effective host plan include the native updater API.

### Mirrors, channels and key rotation

`endpoints` and `publicKeys` each need at least one entry.

```ts filename="tynd.config.ts"
updater: {
  endpoints: [
    "https://updates.example.com/{{channel}}/{{target}}-{{arch}}/manifest.json",
    "https://mirror.example.net/{{channel}}/{{target}}-{{arch}}/manifest.json",
  ],
  publicKeys: ["<new key>", "<old key>"],
  channel: "stable",
}
```

- **Mirrors.** Endpoints are tried in order. The next one is used after a network error, a timeout, or HTTP 5xx / 408 / 429; any other failure (4xx, bad signature, policy refusal) stops immediately. Each entry must be non-empty, unique, and an http(s) URL once placeholders are expanded.
- **Templating.** `{{channel}}`, `{{target}}` (`windows`, `darwin`, `linux`), `{{arch}}` (`x86_64`, `aarch64`) and `{{current_version}}` are replaced (values are percent-encoded; unknown placeholders are left as written).
- **Channel.** `channel` is the default release channel (`stable` when unset; letters, digits, `.`, `_`, `-`, at most 64 characters, not starting with `.`). `updater.setChannel(name)` overrides it and is persisted per install; `updater.getChannel()` reads the current one. Switching drops a pending update found on the old channel.
- **Key rotation.** A signature is valid when any listed key verifies it. To rotate: ship a release that lists old and new keys in `publicKeys`; once installs have updated, sign new releases with the new key; later drop the old key. Keys are pinned at host build time (`TYND_UPDATER_PUBKEY` accepts several keys separated by `,`, `;` or whitespace), so every configured key must match one of them. Keys, proxy and request headers are never taken from renderer calls.
- **Private update servers.** `updater.headers` (for example `{ Authorization: "Bearer ${env:UPDATE_TOKEN}" }`) and `updater.proxy` are set in `tynd.config.ts` only. `${env:NAME}` is expanded at bundle time from the process environment, then `.env`. Headers go only to the origins of the configured endpoints (at least one needs a fixed origin) and are never sent to a redirect target on another origin. The values are baked into the backend bundle, so use read-only tokens; they are never exposed to the renderer. Without `proxy`, `HTTPS_PROXY` / `ALL_PROXY` apply. Limits and rejected header names: [`updater` config](https://kvnpetit.github.io/tynd/docs/v1.0/cli/config#updater).

### Optional server scaffold

```bash
tynd updater server --deploy cloudflare
tynd updater server --deploy vercel
tynd updater server --deploy deno
```

Scaffolds under `.tynd/update-server/<provider>/` and substitutes app id, public key, manifest URL, and provider metadata. Review/deploy generated code with that provider's tools.

## Runtime API

Install the required plugin:

```bash
tynd add updater
```

```ts filename="src/updater.ts"
import { updater } from "@tynd/plugin-updater/client";

export async function checkAndInstall() {
  const info = await updater.checkForUpdates();
  if (!info.available) return;

  await updater.download();
  await updater.apply();
}
```

The high-level API exposes:

- `checkForUpdates()`;
- `getChannel()` / `setChannel(name)`;
- `download()`;
- `apply()`;
- `cancel()`;
- current/available-version and progress getters;
- `checking`, `update-available`, `update-not-available`, `download-progress`, `update-downloaded`, `update-applied`, and `error` events.

`download()` selects a matching patch when `allowPatch` and `preferPatch` permit it and the patch respects `maxPatchSize`. Otherwise it uses the full artifact when `fallbackToFull` is true. `autoApply` can chain a successful download into apply; major-version prompts remain protected by `requireMajorPrompt`.

`autoCheck`, `checkOnStartup` and `autoDownload` are run by the client module: once the main window imports `@tynd/plugin-updater/client`, it starts a periodic check every `checkInterval` (`autoCheck`), runs one check at startup (`checkOnStartup`), and chains a found update into `download()` (`autoDownload`, skipped for a major bump while `requireMajorPrompt` is set). An app that never loads that module never checks. Set them to `false` to drive everything from your own UI as above. `promptStyle` is a policy value exposed to the application; the client does not act on it. The low-level `updaterIpc.startPeriodicCheck()` API is also available.

## Config options

| Field | Default | Meaning |
|---|---:|---|
| `preferPatch` | `true` | Prefer a matching delta route |
| `allowPatch` | `true` | Permit delta downloads |
| `maxPatchSize` | none | Maximum patch bytes; accepts bytes or values such as `5MB` |
| `fallbackToFull` | `true` | Permit full download when no usable patch exists |
| `autoCheck` | `true` | Periodic check every `checkInterval` (main window, client module loaded) |
| `checkInterval` | `12h` | Parsed into runtime seconds |
| `checkOnStartup` | `true` | One check at startup (main window, client module loaded) |
| `autoDownload` | `true` | Download every update a check finds |
| `autoApply` | `false` | Apply immediately after high-level download when allowed |
| `promptStyle` | `modal` | `modal`, `tray`, `silent`, or `manual` policy |
| `requireSignature` | `true` | Signature enforcement; public TS config only allows `true` |
| `requireMajorPrompt` | `true` | Prevent silent auto-apply across a major bump |

## Build release artifacts

Build on each target platform/architecture:

```bash
# Windows or Linux updater payload: raw packed executable is enough
tynd build

# macOS updater payload: signed .app archive is required
tynd build --bundle macos
```

Default platform directories are `release/windows-x64`, `release/windows-arm64`, `release/linux-x64`, `release/linux-arm64`, `release/macos-x64`, and `release/macos-arm64`.

For macOS, the build emits:

```text
<binary>-<version>-macos.zip
```

This archive must contain `<name>.app/Contents/Info.plist`. `tynd updater publish` rejects a missing/ambiguous archive.

## Publish

```bash
tynd updater publish 1.2.3 --target manifest-only
tynd updater publish 1.2.3 \
  --target r2 \
  --bucket my-app-updates \
  --endpoint https://<account>.r2.cloudflarestorage.com \
  --public-base-url https://updates.example.com
```

`tynd updater publish`:

1. validates semver and loads the app-scoped private key;
2. finds exactly one updater payload in every recognized platform directory;
3. signs every payload and records size/BLAKE3 metadata;
4. optionally builds signed BSDIFF+zstd patches for Windows/Linux from archived releases;
5. writes and signs `release/publish/manifest.json` plus `manifest.json.sig`;
6. uploads or leaves files staged, depending on target;
7. archives the release under `~/.tynd/releases/<app-id>/<version>/` (override root with `TYND_RELEASES_ROOT`).

The native `tynd-bundler` shipped beside `tynd` generates patches.

### Channels and staged rollout

```bash
tynd updater publish 1.3.0-beta.1 --channel beta
tynd updater publish 1.3.0 --channel stable --rollout 10   # 10% of installs first
tynd updater publish 1.3.0 --channel stable                # later: everyone
```

`tynd updater publish` always writes `channel` into the manifest: `--channel`, else `TYND_PUBLISH_CHANNEL`, else `updater.channel`, else `stable`. `--rollout` (0-100) is written as `rollout`. A manifest without `channel` (hand-written, for example) is treated as `stable`. Publish one manifest per channel and point each at its own URL with `{{channel}}`. An install is part of a rollout when its stable bucket (0-99, from a random per-install id and the release version) is below the percentage, so the same install stays in or out for a given release. Held-back installs get `available: false`.

### Publish targets

| Target | Behavior | External command |
|---|---|---|
| `manifest-only` | Generate/sign/stage locally | none |
| `github-releases` | Upload release tree to existing `v<version>` release | `gh release upload` |
| `s3` | Recursive upload under `<app-id>/<version>` | `aws s3 cp` |
| `r2` | Same S3-compatible upload with endpoint | `aws s3 cp` |

S3/R2 credentials come from the environment only: `R2_ACCESS_KEY_ID` + `R2_SECRET_ACCESS_KEY`, else `AWS_ACCESS_KEY_ID` + `AWS_SECRET_ACCESS_KEY`, else the AWS CLI profile. Both variables of a pair must be set together.

### Patch selection

| Strategy | Versions considered |
|---|---|
| `same-major` (default) | Every archived version with the current major, capped by `--max-patches` |
| `last-n` | Newest `--max-diffs` versions regardless of major, then hard cap |
| `all` | Every older archived version, then hard cap |

`--max-patches` defaults to 30. `--no-diffs` creates full-payload releases only. macOS is always excluded from binary patch generation.

## Manifest shape

```jsonc
{
  "version": "1.2.3",
  "pub_date": "2026-07-30T12:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "url": "https://updates.example.com/com.example.myapp/1.2.3/windows-x64/MyApp.exe",
      "signature": "<base64 Ed25519 over artifact bytes>",
      "size": 50000000,
      "format": "binary",
      "blake3": "<hex>",
      "patches": [{
        "from": "1.2.2",
        "url": "https://updates.example.com/com.example.myapp/1.2.3/patches/1.2.2-windows-x64.bsdiff.zst",
        "signature": "<base64 Ed25519 over patch bytes>",
        "size": 250000,
        "blake3": "<hex>",
        "compression": "zstd"
      }]
    },
    "darwin-aarch64": {
      "url": "https://updates.example.com/com.example.myapp/1.2.3/macos-arm64/MyApp-1.2.3-macos.zip",
      "signature": "<base64 Ed25519 over archive bytes>",
      "size": 52000000,
      "format": "app-archive",
      "blake3": "<hex>"
    }
  }
}
```

The host derives the manifest-signature URL by appending `.sig`, verifies manifest bytes first, then authorizes only URLs/formats signed into that manifest.

## Local QA server

After publishing locally:

```bash
tynd updater publish 1.2.3 --target manifest-only
tynd updater server --serve --port 3000
```

The server binds `127.0.0.1`, requires `release/publish/manifest.json` and `.sig`, and serves only safe paths below `release/`. It does not synthesize a manifest from loose artifact flags.

## Install semantics

- Windows: stage verified bytes, replace the locked executable through the updater flow, relaunch.
- Linux: replace the verified single-file executable, restore executable permissions, relaunch.
- macOS: require authenticated `app-archive`, reject traversal/links, validate app identity/signature, swap the `.app` with rollback, relaunch after the old process exits.

Always exercise a signed N-1 to N update on each shipped OS/architecture before release.

## Related

- [`tynd updater keygen`](https://kvnpetit.github.io/tynd/docs/v1.0/cli/updater/keygen)
- [`tynd updater publish`](https://kvnpetit.github.io/tynd/docs/v1.0/cli/updater/publish)
- [`tynd updater server`](https://kvnpetit.github.io/tynd/docs/v1.0/cli/updater/server)
- [Updater API](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/updater)
- [Code Signing](https://kvnpetit.github.io/tynd/docs/v1.0/guides/code-signing)