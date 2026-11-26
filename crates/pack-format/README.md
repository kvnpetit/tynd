# tynd-pack-format

Reader and writer for the `TYNDPKG` section appended to packaged Tynd binaries.

## What it is

A packaged Tynd app is one executable: the host binary followed by a `TYNDPKG`
section that holds the Bun binary, the backend bundle, the frontend files,
icons, sidecars and boot metadata. This crate defines that layout once so the
bundler that writes it and the runtimes that read it cannot drift apart. It
depends only on `std`.

Layout: the host bytes, then the section (a `u32` LE entry count, then each
entry as a `u16` LE path length, the path, a `u32` LE data length and the
data), then a 16-byte trailer (`u64` LE section size + the `MAGIC` bytes
`TYNDPKG\0`). The reader also finds the trailer when a Windows Authenticode or
macOS code signature follows it.

## Main API

- `PackReader` — `open()` / `open_path()` locate the section in the running
  executable or a given file; `open_checked()` / `open_path_checked()` tell
  "no pack" (`Ok(None)`) from "damaged pack" (`Err`). Iterate with
  `next_header()`, then `copy_entry_to()` or `skip()` each entry. Every read is
  bounded by the section size.
- `PackEntry` and `write_section_and_trailer(writer, entries)` — write a section
  after the host bytes; `validate_entries` rejects invalid or duplicate paths.
- `validate_subpath` — rejects absolute, UNC, `.` and `..` paths after
  `frontend/` or `sidecar/`.
- Reserved entries: `WEBVIEW2_RUNTIME_PATH_ENTRY` (`webview2.runtime-path`) and
  `DEBUG_FLAG_ENTRY` (`debug.flag`, see `debug_flag_entry` /
  `parse_debug_flag`; a damaged flag reads as off). Boot metadata entries are
  capped at `MAX_BOOT_METADATA_BYTES`.

## How Tynd uses it

- `tynd-bundler` (`crates/bundler`) writes the section during `tynd build`.
- `tynd-runtime` and `tynd-host` read it at startup (`embed.rs`).
- `crates/cli` opens packs to check them during signing and `tynd updater release`.

See [TYNDPKG](https://kvnpetit.github.io/tynd/docs/v1.0/concepts/tyndpkg) for
the entry list.

## Validation

```bash
cargo test -p tynd-pack-format
```

## License

Apache-2.0
