# 🆘 Troubleshooting

Common problems and how to fix them. If you don't find yours here, run `tynd info --verbose` and open an [issue](https://github.com/kvnpetit/tynd/issues) with the output.

---

## 📦 Install & setup

### `tynd-full` / `tynd-lite` binary not found

The CLI looks in this order: workspace `target/release` -> `target/debug` -> `node_modules/@tynd/host/bin/<plat>-<arch>` -> system `PATH`.

**Inside the Tynd monorepo (contributors):**
```bash
cargo build --release -p tynd-full
cargo build --release -p tynd-lite
```

**In a user project:**
```bash
bun add @tynd/host
# postinstall downloads the matching binary for your OS/arch from GitHub Releases
```

If `@tynd/host` is installed but no binary is present under `node_modules/@tynd/host/bin/`, the postinstall probably failed — check your internet connection, proxy, and then:
```bash
bun install --force
```

### Postinstall can't download the binary

`@tynd/host`'s postinstall fetches release assets from `github.com/kvnpetit/tynd/releases`. Requirements:
- The repo must be **public** at the time of download.
- Your machine must have outbound HTTPS to `github.com` and `objects.githubusercontent.com` (GitHub uses signed S3 redirects).
- Corporate proxies: set `HTTPS_PROXY=http://proxy:port` before `bun install`.

Temporary workaround: build from source with `cargo build --release -p tynd-{full,lite}` and set `PATH` to include `target/release/`.

### `bun: command not found`

Tynd is Bun-first. Install Bun once:
```bash
curl -fsSL https://bun.sh/install | bash   # macOS/Linux
powershell -c "irm bun.sh/install.ps1 | iex"  # Windows
```

End users of your built app do **not** need Bun — the runtime is bundled into the binary.

---

## 🪟 WebView & platform quirks

### Windows: nothing opens, app exits silently

WebView2 runtime is missing. Windows 11 ships it; Windows 10 usually has it via Edge updates, but some locked-down VMs don't. Install the Evergreen Bootstrapper:
- https://developer.microsoft.com/microsoft-edge/webview2/

### Windows: SmartScreen "Unknown publisher"

Your binary isn't code-signed yet. Either click "More info -> Run anyway" for local testing, or sign it — see [SIGNING.md](./SIGNING.md).

### macOS: "can't be opened because it is from an unidentified developer"

Gatekeeper rejects unsigned apps. Either:
- Right-click the `.app` -> Open -> Open (one-time override), or
- Sign and notarize — see [SIGNING.md](./SIGNING.md).

### Linux: `error while loading shared libraries: libwebkit2gtk-4.1.so.0`

Missing WebView dependencies. On Debian/Ubuntu:
```bash
sudo apt install libwebkit2gtk-4.1-0 libjavascriptcoregtk-4.1-0 libsoup-3.0-0
```

For building from source:
```bash
sudo apt install libgtk-3-dev libwebkit2gtk-4.1-dev \
  libjavascriptcoregtk-4.1-dev libsoup-3.0-dev
```

Fedora:
```bash
sudo dnf install gtk3-devel webkit2gtk4.1-devel libsoup3-devel
```

---

## 🛠️ CLI errors

### `<framework> detected — incompatible with server-side frameworks`

Tynd owns the HTTP layer (`tynd://` custom scheme in prod, dev server URL in dev). SSR frameworks that need Node/Bun on the end-user's machine are blocked: Next.js, Nuxt, SvelteKit, Remix, Gatsby, SolidStart, Angular Universal, Qwik City, Astro, TanStack Start, Vike.

Use the pure-SPA variant: plain React (Vite), plain Svelte, plain Solid, etc.

### `backend is not a string` / config validation errors

`tynd.config.ts` is validated with [valibot](https://valibot.dev). The error message lists the offending field. Required: `runtime` (`"lite"` or `"full"`). Most others are optional with sane defaults — see `TyndConfig` in [@tynd/cli](./packages/cli/README.md).

### `tynd dev` shows a blank window, no errors in terminal

Three likely causes:
1. The dev server hasn't started yet — wait ~2s. Tynd probes up to 60 seconds before giving up.
2. Custom `devUrl` is wrong. Check `tynd info` to see what Tynd is targeting.
3. The frontend itself throws on load. Open devtools with `tynd dev --verbose` or add `window.addEventListener("error", console.error)` and re-run.

### `tynd build --bundle` fails with `rpmbuild: command not found`

RPM is the only bundler Tynd doesn't auto-download — it's too platform-specific. Install it:
```bash
sudo apt install rpm              # Debian/Ubuntu
sudo dnf install rpm-build        # Fedora/RHEL
```

Or build only the formats you need: `tynd build --bundle app,dmg,deb,appimage,nsis,msi` (drops `rpm`).

### React Fast Refresh triggers full reloads

React Compiler (in React 19+) conflicts with Vite's Fast Refresh. Disable the compiler, or accept full reloads for now — see the note in [FRAMEWORKS.md](./FRAMEWORKS.md).

---

## ⚡ Runtime errors

### `fs.readBinary` / `compute.hash` fails with "unknown binary route"

You're hitting the `tynd-bin://` custom protocol and the request has a typo. Expected routes: `fs/readBinary`, `fs/writeBinary`, `compute/hash`, `compute/compress`, `compute/decompress`. If you're writing custom client code, prefer the published wrappers in `@tynd/core/client`.

### `sql.open(path)` silently uses in-memory mode

Empty string or `":memory:"` opens an in-memory DB. Anything else is treated as an on-disk path. If your data isn't persisting, make sure you pass an absolute path or one resolved against `os.dataDir()`.

### Events don't arrive in the frontend

- Make sure `app.start()` was called in the backend.
- Emitters returned by `createEmitter` must be **exported** from `backend/main.ts` — the frontend proxy needs to see them at bundle time for `api.on("name", ...)` to work.
- In lite mode, events require the host's `__tynd_emit__` shim — running a lite bundle without `tynd-lite` produces no events.

### `singleInstance.acquire()` returns `{ acquired: false }` even alone

Stale lock. The OS-level mechanism (named pipe on Windows, abstract socket on Linux, `CFMessagePort` on macOS) usually self-heals on process exit. If a crash left a zombie:
- Windows: nothing to clean up — Windows frees named pipes when the owning process dies.
- Linux: the abstract socket is auto-released by the kernel.
- macOS: `killall MyApp` or reboot.

---

## 🧭 Performance / sizing

### Binary is bigger than `@tynd/host` reports

The stock host is ~6.5 MB. `tynd build` appends the packed frontend + backend + (in `full`) zstd-compressed Bun (~37 MB). Expected totals: ~8-10 MB for `lite` apps, ~44 MB for `full`.

### `tynd dev` feels slow to start

First run: backend + frontend rebuild from scratch (~2-5 s depending on size). Subsequent runs hit the cache under `.tynd/cache/` and start in <1 s. If caching isn't working, check `tynd info` for the cache directory and inspect it.

### Dialogs / file pickers stutter

The OS-call pool caps concurrent threads (see `os::call_pool`). If you're spamming 50+ simultaneous dialogs, they queue. That's usually what you want — rate-limit your own calls on the client side.

---

## 📦 Packaging

### NSIS installer wants admin privileges

Default is `currentUser` (no UAC). If you're being prompted, either:
- You have `bundle.nsis.installMode: "perMachine"` in `tynd.config.ts` — remove it.
- Your `%LOCALAPPDATA%\Programs` is restricted — check Group Policy.

### `.deb` fails to install with "package depends on..."

Tynd's `.deb` generator lists WebKitGTK as a dependency. On systems without the GNOME stack (server installs, minimal distros), install `libwebkit2gtk-4.1-0` first.

### AppImage won't run on older glibc

`appimagetool` bundles the AppRun shim but not glibc. The AppImage links against the glibc version used at build time. Build on the oldest supported system (e.g. Ubuntu 20.04 LTS) to maximise compatibility.

---

## 🆘 Still stuck?

- `tynd info --verbose` — shows everything Tynd sees about your environment
- `tynd validate` — checks config + binary availability
- [GitHub Issues](https://github.com/kvnpetit/tynd/issues) — include the output of the two commands above
