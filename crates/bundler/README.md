# tynd-bundler

Native installer, signing, patch, and pack helper used by the Rust `tynd` CLI.

It builds on maintained Rust crates:

- [`tauri-bundler`](https://crates.io/crates/tauri-bundler) for MSI, NSIS,
  DMG, `.app`, `.deb`, `.rpm`, and AppImage output;
- [`apple-codesign`](https://crates.io/crates/apple-codesign) for Mach-O,
  `.app`, DMG, and `.pkg` signing plus Apple notarization;
- `bidiff` and `zstd` for updater patches;
- `resvg`, `usvg`, `image`, and `ico` for deterministic icon conversion.

The crate is a member of the root Cargo workspace. Its release-time
dependencies are never linked into `tynd-host` or `tynd-runtime`.

## Build

From the repository root:

```bash
cargo build --release -p tynd-bundler
```

The binary is written to `target/release/tynd-bundler[.exe]`. A `tynd` built
from the same checkout finds it there.

## Usage

`bundle`, `codesign` and `pack` accept a JSON config file, or `-` to read JSON
from stdin; the other subcommands take flags:

```bash
tynd-bundler bundle bundle.json
tynd-bundler codesign signing.json
tynd-bundler pack pack.json
tynd-bundler patch --old <PATH> --new <PATH> --output <PATH>
```

The JSON requests are the types of
[`tynd-bundle-contract`](../bundle-contract), which `tynd-cli` uses to write
them. Artifact paths are printed on stdout, one per line.

Normal users do not invoke this executable directly. `tynd build`,
`tynd updater publish`, and related Rust commands resolve it through `@tynd/tooling`.

## Distribution

Published releases stage `tynd-bundler` beside `tynd` and `tynd-mcp` in each
`@tynd/tooling-<os>-<arch>` platform package. npm optional dependencies select
the current target. There is no install hook and no GitHub binary download.

## Subcommands

| Subcommand | Responsibility |
| --- | --- |
| `bundle` | Native installer and application bundle generation |
| `codesign` | macOS signing, notarization, and stapling |
| `patch` | Binary patch generation with optional zstd compression |
| `archive-app` | Zip a signed `.app` for the updater |
| `dmg-from-app` | DMG from an already signed and stapled `.app` |
| `pack` | Self-extracting package generation |
