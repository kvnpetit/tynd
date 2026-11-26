---
title: Security
description: "The security model — CSP defaults, scheme allowlists, structural exposure surface, and OS-backed secret storage."
---

# Security

Tynd's security model is **structural + conservative-by-default**. The exposure surface of your app is the exported module — code and policy can't drift apart — and the WebView runs with an auto-injected CSP.

## Threat model

Tynd targets a single-user desktop environment. The primary concerns:

- **Untrusted content injection** into the WebView (XSS from user-supplied HTML, data URLs, `postMessage`-based smuggling).
- **Local-machine attackers** sniffing IPC traffic if it were on TCP. *(It isn't — see below.)*
- **Supply-chain compromise** of a bundled `.exe` (solved by code signing + notarization).
- **Update-channel hijacking** (solved by Ed25519-signed auto-updates).

What's **not** in scope:

- Protecting app secrets from a user with admin access on their own machine. Disk-encryption + OS-level isolation are the right answer, not app-level obfuscation.

Tynd has two startup-only policy layers: [`security`](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/security) for path/URL/process rules, and structured `capabilities` for per-command, per-window, per-webview, per-origin, and per-platform grants.

## Transport — no TCP, no firewall prompt

RPC and frontend asset serving never touch a TCP port. Everything rides:

- **`tynd://localhost/<path>`** (WebView host custom scheme) for frontend assets.
- **`tynd-bin://localhost/<api>/<method>?<query>`** (WebView host custom scheme) for binary IPC.
- **`window.ipc.postMessage`** for JSON IPC.
- **`evaluate_script`** for backend → frontend pushes.

Consequence: no loopback port, no firewall prompt on first launch, and no MITM window for another process on the same machine that could listen on a socket.

## CSP — auto-injected

Every HTML response served through the `tynd://` scheme carries a baseline Content-Security-Policy header that:

- Blocks **inline scripts** (no `<script>alert()</script>` from smuggled HTML).
- Disables `frame-src` and `object-src`.
- Restricts `connect-src` to same-origin (`tynd://localhost`) plus HTTPS/WSS.
- Allows workers from `'self'`, `tynd:` and `blob:` (`worker-src`); `workers.spawn` starts its worker from a `blob:` URL.

A page can add its own `<meta http-equiv="Content-Security-Policy">` tag. The browser enforces both policies, so a meta tag can only **tighten** the baseline, never loosen it.

`loadHtml` documents get the same baseline as a `<meta http-equiv="Content-Security-Policy">` tag that Tynd inserts as the first element of the HTML (after a leading `<!DOCTYPE>`), because the platform loads that HTML without response headers. The document commits as `about:blank`, so the policy names the bundled origin (`tynd:`, and `http://tynd.localhost` on Windows) instead of relying on `'self'`. Inline `<script>` blocks do not run in `loadHtml` content: load scripts from your bundled assets.

Dev-server pages (`tynd dev`) get no CSP from Tynd; the host logs a warning at startup. Test the policy on a bundled build.

If your custom policy sets `script-src`, `default-src` or `worker-src`, include `worker-src 'self' blob:`, or [`workers.spawn`](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/workers) fails because the browser blocks the `blob:` worker. Workers fall back to `script-src` when `worker-src` is absent, and `script-src 'self'` does not allow `blob:`.

<Callout type="warning">
  **Don't relax CSP without a reason.** Every loosening (`'unsafe-inline'`, `'unsafe-eval'`, broad `connect-src *`) widens the attack surface for any bug that injects HTML into your WebView.
</Callout>

## `shell.openExternal` — scheme allowlist

```ts
shell.openExternal(url);   // opens in the default browser
```

Rust-side, only **`http://`**, **`https://`**, and **`mailto:`** schemes are accepted. Passing `file://`, `javascript:`, `data:`, or a registered custom scheme throws.

## `shell.openPath` — absolute paths only

```ts
shell.openPath("/Users/me/document.pdf");   // opens in OS default handler
```

Passed to the OS's "open with default app" handler, which decides what handles the file. The host first refuses launcher, script, installer and shortcut types (`.exe`, `.bat`, `.ps1`, `.lnk`, `.app`, `.pkg`, `.sh`, `.desktop`, ...) on every OS, matching the final extension of the file name (`script.exe` is refused, `notes.exe.txt` opens with the `.txt` handler). See the [shell API](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/shell#openpathpath-promisevoid) for the full list.

## Custom URL schemes — reserved list

If you declare `protocols: ["myapp"]` in `tynd.config.ts`, the following are rejected at config-validation time:

- `http`, `https`, `file`, `ftp`, `mailto`, `javascript`, `data`, `about`, `blob`, `tynd`, `tynd-bin`

See [Deep Linking](https://kvnpetit.github.io/tynd/docs/v1.0/guides/deep-linking).

## Secret storage — `keyring` > `store`

For anything sensitive (OAuth tokens, API keys, session cookies, passwords):

- **`keyring`** — OS-encrypted credential storage. Keychain (macOS), Credential Manager + DPAPI (Windows), Secret Service / GNOME Keyring / KWallet (Linux). Secrets are encrypted at rest with the user's login credentials.
- **`store`** — plain JSON k/v at `config_dir()/<app-id>/<ns>/store.json`. **Readable by any process with user-level access** — use only for non-sensitive preferences.

Install the required plugins:

```bash
tynd add keyring store
```

```ts
import { createStore } from "@tynd/plugin-store/client";
import { keyring } from "@tynd/plugin-keyring/client";

// Sensitive — use keyring
await keyring.set({ service: "com.example.app", account: "alice" }, "s3cr3t-token");

// Non-sensitive — use store
const prefs = createStore("com.example.app");
await prefs.set("theme", "dark");
```

See the [Persistence guide](https://kvnpetit.github.io/tynd/docs/v1.0/guides/persistence).

## Code signing — trust the binary

Unsigned binaries trigger SmartScreen warnings (Windows), Gatekeeper quarantine (macOS), and download flags in Chrome/Edge. Tynd ships built-in signing via the `bundle.sign` block in `tynd.config.ts`:

- **Windows** — `signtool.exe` (SHA-256 + timestamp server).
- **macOS** — built-in Rust signing with a Developer ID `.p12` / `.pfx`, plus optional App Store Connect API-key notarization.

See the [Code Signing guide](https://kvnpetit.github.io/tynd/docs/v1.0/guides/code-signing).

## Auto-update — Ed25519 signatures

The updater downloads an artifact and **verifies an Ed25519 signature** over the raw file bytes before installing. The public key is supplied by the app (typically baked in at build time), so a compromised manifest server can only redirect to a URL whose bytes still have to verify against the local pubkey.

- Tynd signs its own update manifest. Its platform map follows the common Tauri shape, but Tynd extensions make the manifests non-interchangeable.
- The CLI ships `tynd updater keygen` / `tynd updater sign` so you can produce signed manifests without third-party tools.
- Native Rust Ed25519 signing and verification use raw 32-byte public keys and raw 64-byte signatures, with no external signing runtime.

See the [Auto-Updates guide](https://kvnpetit.github.io/tynd/docs/v1.0/guides/auto-updates).

## OS-level `process.exec` / `execShell`

```ts
process.exec("git", { args: ["status"] });        // direct exec — no shell interpolation
process.execShell("ls -la");                       // shell=true — cmd.exe / sh, needs shellExecAllowedCommands
```

`process.exec` is the **safe default** — arguments are passed as an array, no shell interpolation. `process.execShell` is opt-in (`security.shellExecAllowedCommands` must list the first token), refuses command lines with shell metacharacters (including `% ^ !`) and refuses a caller-supplied `env`. Validate untrusted inputs yourself.

Environment variables passed to `exec` / `spawn` are checked against a denylist of loader and interpreter hooks (`PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `GIT_CONFIG*`, `PYTHONPATH`, `NPM_CONFIG_*`, ...). A denied key makes the call fail. See [process](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/process#options).

## Structural security — export = exposure

The frontend can only call what the backend **explicitly exports**:

```ts
// backend/main.ts
export async function greet(name: string) { … }   // callable from frontend
export const events = createEmitter<…>();          // subscribeable from frontend

async function internal() { … }                     // NOT callable — not exported
```

Backend RPC remains structural: the exported TypeScript module defines callable application functions. Native renderer commands require explicit startup capability grants.

Consequences:

- **Safe default** — you have to opt a function in (via `export`) to make it reachable. Forgetting to export something is a "safer than intended" outcome, not the other way around.
- **Separate trust boundaries** — exports govern renderer → Bun RPC; capabilities govern renderer → native commands.
- **Grep-auditable** — search for `export async function` or `export const events =` to enumerate the entire surface.

## Capability ACL (`security`) — opt-in

```ts
import { homedir } from "node:os";
import { join } from "node:path";
import { app } from "@tynd/core";

const projectsDir = join(homedir(), "Projects");

app.start({
  security: {
    fs:   { allow: [`${projectsDir}/**`], deny: ["**/.env"] },
    http: { allow: ["https://api.myapp.com/**"] },
    defaultDeny: true,
  },
});
```

Deny beats allow, and `fs.allow` rules are how a renderer reaches anything outside the app's own directories. Globs: `*` (intra-segment) + `**` (cross-segment). Paths normalized; case-insensitive on Windows (deny rules on macOS too). Only an allow rule starting with an absolute directory reaches outside the app's directories, and only below it; `**/*.pdf` just filters inside them. The trusted backend installs this policy before WebView creation; renderer IPC cannot mutate it. Full reference: [security API](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/security).

<Callout type="warning">
  **With no `security` policy configured, the `http` scope is open.** Any granted native command can then reach any URL. File-system access is never open: renderers and plugins reach only the app's own data, config, cache and temp directories (plus the working directory under `tynd dev`), and anything else needs an `fs.allow` rule or a runtime grant. Credential stores, browser profiles and Tynd's own grants file are refused even then. See [default filesystem scope](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/security#default-filesystem-scope). `process.exec` / `spawn` stay refused until `processAllowedBinaries` lists the binary. `tynd validate` and `tynd build` warn when no policy is configured. Configure `allow` lists (and ideally `defaultDeny: true`) before shipping.
</Callout>

`http` allow and deny patterns are matched against the **parsed** URL: scheme and host are lowercased and a default port (`:443` for `https`, `:80` for `http`) is dropped before matching, so `HTTPS://API.Example.com:443/v1` matches `https://api.example.com/v1`. Once a policy is configured, URLs with userinfo (`user@host`), backslashes or control characters, or that fail to parse, are refused. Matching is textual: a deny rule on a host or IP is **not** an SSRF boundary, because a DNS name that resolves (or rebinds) to a loopback or private address is not caught by it. IPv4-mapped IPv6 hosts (`[::ffff:127.0.0.1]`) are rewritten to plain IPv4 before matching.

## Local and remote webviews

Several rules depend on whether a webview shows app-local or remote content.

- **Navigation refuses URLs with userinfo.** An `http(s)` navigation target with `user@host` or `user:pass@host` is refused, even without a `security` policy, so `http://u@tynd.localhost/` is never treated as app content.
- **App-local content** is the bundled `tynd://` scheme (and, on Windows only, its WebView2 form: exactly `http://tynd.localhost` on the default port; `https://tynd.localhost`, another port or a look-alike host such as `tynd.localhost.example` is remote) and, under `tynd dev`, the exact origin of the dev server. Any other `localhost` or loopback address, a different port included, is remote; in a packaged app no loopback origin is local.
- **Trust is granted when a document commits.** Starting a navigation to a remote page already marks the webview remote, but a webview only becomes local again once a local document has committed in its main frame. A navigation that is cancelled, redirected or turned into a download never grants local trust.
- **Frames are judged by their own origin.** A `blob:` frame inherits the origin of the page that created it (a `blob:` of a remote page is remote). `data:` and `srcdoc` documents, any other `about:blank` frame, and `blob:` URLs without a real origin are opaque: they get no IPC at all. Only the main document may be `about:blank`.
- **The IPC bridge lives in the top frame.** `window.__tynd__` is installed in the main frame, and in a subframe only when the subframe's own location is app content; a remote or opaque iframe gets no bridge. Each webview has two random 256-bit IPC tokens, held by the bridge in a closure captured before any page script runs; the host drops every IPC message that carries neither, so an iframe that calls the platform message handler directly (`window.webkit.messageHandlers.ipc`, `window.chrome.webview`) is ignored on every platform. This matters most on Linux, where WebKitGTK reports the main-frame URL for every message: without the tokens a cross-origin iframe would post with the trust of the page that embeds it.
  - The **remote token** is part of the bridge script, which every document of the webview receives, remote pages included. A message carrying it never gets more than remote-origin privileges: when the main frame shows app content, it gets the opaque `null` origin, which matches no capability.
  - The **app token** is never part of any script. At start-up the bridge of an app document in the main frame (`tynd://`, `http://tynd.localhost` on Windows, the `tynd dev` server, or `about:blank` for the placeholder page and `loadHtml`) asks the host for it; the host answers only while it tracks the main frame as app content, by evaluating the answer in the main frame behind a check of that document's own `location`. Messages carrying the app token are dropped whenever the main frame is not app content (a navigation to a remote page counts from its start), and the app token is re-minted each time a remote document commits. So a remote page that was shown in the main frame earlier, and kept what it could read, cannot later post from an iframe with app privileges. Calls made before the token arrives are queued.

  Limits: a frame with the **same origin** as the top page can script that page (`parent.__tynd__`) and so calls with the page's trust; same origin is the same trust boundary on the web, so embed only content you trust at your app's origin. An app-content subframe (Windows runs the bridge in every frame) only holds the remote token, so its calls get no app privileges; call through `parent.__tynd__` instead. A cross-origin subframe on Windows returns from the bridge script before reading any token, but the script text it was injected with still contains the remote token, which grants nothing beyond that frame's own origin.
- **OS and plugin events reach local webviews only by default.** No event is marked safe for remote pages, so `clipboard:change`, `app:second-launch`, `app:open-url`, `deep-link:url`, `window:drag-enter`, `window:drop` and every native-plugin event (streams included) go to app-local webviews only. A remote page never sees clipboard text, forwarded argv, deep-link URLs (which can hold OAuth codes), dropped file paths or plugin data.
- **Renderer-owned resources belong to one webview.** File watches and handles (`fs`), spawned processes, sockets, keep-awake requests, global shortcuts, scheduled notifications and staged updates created by a renderer are owned by the calling webview. Their events (`fs:change`, process output and exit, socket events, `shortcut:triggered`) go to that webview alone, not to other local webviews. Ownership is the webview label, the document (it changes on every navigation or reload) and the frame origin, so a remote iframe inside a local page cannot use the page's resources. Only the owner (or the backend) can use or cancel them: another webview's id answers like an unknown id, `isRegistered` returns `false`, and `shortcuts.unregisterAll` and `websocket.list` see the caller's own entries, plus backend-created ones for an app-local webview. A remote webview that holds the capability grant to create such a resource gets the events of its own resources, and only those. A resource created from the backend reaches app-local webviews only and can be listed and used by them, never by a webview showing remote content. A watch, handle, socket, shortcut or keep-awake request that completes after its document is gone fails with "the document that requested it is gone"; a process, notification or staged update registered that late is re-owned by the backend. Plugin events stay local-only.
- **Reload, navigation and close release them.** When a webview commits a new document or closes, its watches, handles, sockets, keep-awake requests and shortcuts are released (the JS that would close them is gone). Spawned processes, scheduled notifications and staged updates are not stopped: they are re-owned by the backend, so a process keeps running and its output now goes to app-local webviews, and a remote document can no longer control them. A spawned process that the renderer never activated (for example it reloaded right after `spawn`) is killed after a 30 s activation timeout.

## Per-context command capabilities

Native OS and plugin commands called from a renderer are deny-by-default (`backend:`, `window:`, `tray:` and `menu:` commands are the opt-in exceptions, see below). `capabilities` adds only explicit grants:

```ts
import { app } from "@tynd/core";

app.start({
  capabilities: [{
    identifier: "main-window",
    windows: ["main"],
    permissions: [{
      identifier: "main-native-access",
      commands: {
        allow: ["window:setTitle", "dialog:openFile", "fs:readText"],
        deny: ["process:execShell"],
      },
    }],
  }],
});
```

Deny rules win. `windows`, `webviews`, `local`, `remote.urls`, and `platforms` narrow where a grant applies. Omitting `capabilities` or passing `[]` grants nothing. The trusted Bun backend sends grants before any WebView exists; the renderer cannot load, add, or remove them.

Standalone plugins whose native crate uses `#[plugin_impl]` also publish named command
permissions. A method such as `start_capture` becomes `startCapture` on the
wire and can be granted as `audio:allow-start-capture`:

```ts
app.start({
  capabilities: [{
    identifier: "audio-window",
    windows: ["main"],
    permissions: [
      "audio:allow-list-devices",
      "audio:allow-start-capture",
    ],
  }],
});
```

Every generated manifest includes per-command `allow-*` / `deny-*`
permissions, `allow-all` / `deny-all` sets, and a `default` alias for
`allow-all`. Permission sets may contain other sets; unknown references,
foreign-manifest references, name collisions, and cycles reject startup.
Deny still wins after set expansion. Core host commands use inline
`commands.allow` / `commands.deny` entries.

## Backend functions and window / tray / menu commands

Two more command families are checked by the same `capabilities`, but they are **opt-in**: nothing changes until your capabilities mention them.

| Command | Covers | Checked when |
|---|---|---|
| `backend:<fn>` | Your exported backend functions called from a renderer | Any `backend:` permission is declared |
| `window:<method>` | `tyndWindow.*` / `tyndWebview.*` calls (`window:create`, `window:setTitle`, ...) | Any `window:` permission is declared |
| `tray:<method>` | `tray.set*` calls | Any `tray:` permission is declared |
| `menu:<method>` | `menu.update` / `menu.setItem*` calls | Any `menu:` permission is declared |

Rules:

- **Not declared, allowed.** If no loaded capability mentions the family (a `deny` entry counts as mentioning it), every call in that family is allowed.
- **Declared, deny by default.** Once any entry exists in a family, a call is allowed only if a capability matching the calling window, webview and origin grants it. Rejections use the same error as other denied native commands.
- **Sensitive commands are the exception.** The four webview cookie commands (`window:getWebviewCookies`, `window:setWebviewCookie`, `window:deleteWebviewCookie`, `window:clearWebviewCookies`), `window:evaluateScript`, `window:clearBrowsingData` and silent printing (`window:printPage` with `silent: true`; the print dialog form stays open) always need an explicit grant.
- **Remote origins are the other exception.** A call that comes from a webview showing remote content (an allow-listed remote site, or a remote iframe) always needs an explicit grant for `backend:`, `window:`, `tray:` and `menu:` commands, even when no capability mentions that family. Grant it through `remote.urls`. App-local pages keep the "not declared, allowed" rule.
- **Fail closed.** If a capability manifest fails to load or validate, every `backend:`, `window:`, `tray:` and `menu:` call is denied, and so are the other gated commands, until a valid load. Look for the host's ACL error in the logs.

```ts
app.start({
  capabilities: [{
    identifier: "main-backend",
    windows: ["main"],
    permissions: [
      "backend:listFiles",            // shorthand for an allow of backend:listFiles
      { identifier: "main-native", commands: { allow: ["window:setTitle"] } },
    ],
  }, {
    identifier: "admin-backend",
    windows: ["admin"],
    permissions: ["backend:*"],       // every backend function
  }],
});
```

With this configuration the `main` window can call `listFiles` and nothing else from the backend, and only `admin` can call any function. The `backend:` check uses the real window and webview labels and origin, which the host attaches itself; a renderer cannot supply them.

Inside the backend, the same identity is available as `getCallContext()` and to `app.use` middleware, so you can add finer rules in code. See [Backend API](https://kvnpetit.github.io/tynd/docs/v1.0/api/backend#call-context-and-middleware).

## Runtime scope grants

The `fs` and `http` scopes can be widened after startup, for named paths and URL patterns only: `capabilities.allowPath` / `allowUrl` from the backend (the counterpart of Tauri's `fs` scope `allow_file` and persisted scope), or a file dialog opened with `grantAccess: true` (the user's pick is the consent). Renderers and native plugins cannot call `capabilities.*`, deny rules always win, paths are resolved by the OS when granted, read-only grants admit reads only, and `security.persistGrants` keeps grants across restarts with every entry re-resolved on load. Reference: [Runtime grants](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/security#runtime-grants-backend).

## Remote content: hard limits

A capability can name a remote origin with `remote: { urls: ["https://*.example.com/**"] }` (and `local: false` to keep it off app content), a window or webview label glob with `windows: ["main", "settings-*"]` / `webviews`, and a platform with `platforms: ["windows", "macOS", "linux"]`. Remote content gets no native command until a capability names its origin. Even then, these stay refused to a webview showing remote content:

- `security.configure`, the runtime grant methods, `acl.generateSchema`, `updater.configure` and `app.setInfo({ version })` (refused to every renderer, local or remote).
- Driving another window: `window.*` calls with another window's label, window messages (`emitTo`), and `tray.attachWindow` / `tray.positionWindow` with another window's label.
- Session commands on another webview, and cookies outside the page's own origin.
- Silent printing, and `grantAccess` on file dialogs.
- Backend-owned resources (watches, processes, sockets...) and resources of another origin or document.
- OS and native-plugin events, and `app.protocol` schemes.

## Hardening compared with Electron and Tauri

| Electron fuse / `webPreferences` | Tynd |
|---|---|
| `nodeIntegration`, `contextIsolation`, `runAsNode` | Not applicable: the renderer has no Node or Bun. The backend is a separate process reached only through the host. |
| `webSecurity`, `allowRunningInsecureContent` | Always on; there is no option to turn the web engine's origin checks off. |
| `devTools` | Enabled only by `tynd dev` (the unpacked runtime's `--debug`). A packaged app never enables them. |
| `will-navigate` allow-list | Main-frame navigation to a remote page needs a configured `security.http` policy that allows it; `javascript:`, `data:`, `file:` and URLs with credentials are always refused. |
| `setWindowOpenHandler` | The `newWindow` window option: `"deny"`, `"external"` or `"allow"` (an unmanaged popup without the IPC bridge, for URLs that pass the navigation policy). |
| Default CSP | Bundled `tynd://` HTML always gets a CSP header with `frame-src 'none'` and `object-src 'none'` (see [CSP](#csp--auto-injected)); a `<meta>` CSP can only tighten it. `loadHtml` content gets the same policy as a `<meta>` tag; dev-server content gets none. |
| IPC sender checks | The host classifies the frame that posted each IPC message, on every call: a remote iframe is judged by its own origin, opaque frames (`srcdoc`, `data:`) get nothing. Not only at page load. Messages without one of the webview's IPC tokens are dropped, and only the app token, handed to app documents in the main frame, earns app trust; this covers Linux, where the frame URL is not reported (see [Local and remote webviews](#local-and-remote-webviews)). |
| Frozen preload API | `window.__tynd__` is frozen and cannot be reassigned or redefined. Authorization never depends on it: the host checks every message. |

## Isolation pattern: why Tynd does not ship one

Tauri's isolation pattern runs a sandboxed iframe that encrypts IPC messages before they leave the page, so that a compromised frontend dependency cannot forge calls the isolation script would reject. In Tynd that layer would not add a boundary:

- The page and the isolation iframe would run in the same document context of the same webview; script injected into the page (XSS, a malicious dependency) can still call `window.__tynd__` directly or drive the iframe the way the app does, so encryption only proves that the message went through the app's own JS.
- What a call may do is decided in Rust after the message arrives: the calling frame's origin, the window and webview labels and the capability set are checked on every message, and argument validation runs in the host. The only secrets the renderer holds are the IPC tokens, which prove which bridge a message came from, not that it is allowed: the host re-checks every privilege.
- The equivalent filter in Tynd is server-side and cannot be bypassed from the page: narrow `capabilities` per window and origin, `backend:` permissions, and `app.use` middleware for argument-level rules on backend calls.

## What Tynd does NOT offer (yet)

- **Context isolation** (renderer ↔ preload boundary) — N/A because there is no Node/Bun in the renderer; the only injected globals are the Tynd IPC shims (`window.__tynd__` is frozen).
- **Renderer sandbox mode** — no configurable sandbox; the WebView runs with the OS's default web-content permissions.
- **Permission request handlers** (camera, mic, geolocation prompts) — the WebView handles them per the OS default, Tynd doesn't intercept.

## Security checklist for shipping an app

<Steps>

### Sign and notarize

Configure `bundle.sign` in `tynd.config.ts` (Windows `signtool` + macOS P12 signing + optional notarization). Unsigned binaries hurt user trust and trip every OS defense mechanism.

### Bake your updater public key

Generate a keypair with `tynd updater keygen`. Store the private key **offline**; commit only the `.pub`. Bake the pubkey into your app source. Every `tynd updater sign` step produces a signature; only signatures verifiable against the baked pubkey install.

### Keep CSP tight

Don't add `'unsafe-inline'` / `'unsafe-eval'` unless you know why you're adding them. If you must, scope it to a single route via `<meta http-equiv>`.

### Use `keyring` for secrets

Never put tokens / passwords / session cookies in `store`. Use `keyring`.

### Validate user-supplied paths

If you let a user pass a path to `fs.readText` / `process.exec`, validate the path stays inside a directory you control. Don't trust normalization — canonicalize first and check for prefix.

### Use `process.exec`, not `execShell`, with user input

`execShell` runs through `cmd.exe` / `sh` — any unquoted user input is a shell injection. Default to `process.exec("git", { args: [...] })` with arguments as an array.

</Steps>

## Next

- [Code Signing guide](https://kvnpetit.github.io/tynd/docs/v1.0/guides/code-signing)
- [Auto-Updates guide](https://kvnpetit.github.io/tynd/docs/v1.0/guides/auto-updates)
- [keyring API](https://kvnpetit.github.io/tynd/docs/v1.0/api/os/keyring)
