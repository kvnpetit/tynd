# 📊 Tynd vs Tauri v2 vs Wails v3 vs Electron

> **Exhaustive feature matrix across 39 categories — 512 rows.**
> Last updated: April 19, 2026 · Desktop only (mobile features marked 📱)

## Tynd in one paragraph

Tynd is a desktop-app framework with a **TypeScript backend** and a native WebView front (wry + tao). It ships **two runtimes from the same TS source**: `lite` (~6.5 MB, embedded QuickJS, Web-standards-only JS surface) and `full` (~44 MB, Bun subprocess, full Node/Bun surface). All OS APIs (`fs`, `http`, `websocket`, `sql`, `process`, `compute`, `terminal`, `tray`, `dialog`, `clipboard`, `shell`, `notification`, `tyndWindow`, `menu`, `sidecar`, `singleInstance`, `store`, `workers`) live in Rust and are identical across both runtimes — what differs is only the JS surface available inside backend code. The matrix below is ordered by category; each row compares Tynd against Tauri v2, Wails v3, and Electron.

---

> **Note:** Tynd's Rust-backed OS APIs (`fs`, `http`, `websocket`, `sql`, `process`, `store`, `compute`, `workers`, `terminal`, `sidecar`, `singleInstance`, `dialog`, `clipboard`, `shell`, `notification`, `tray`, `tyndWindow`, `menu`) land identically on `lite` and `full`. The `lite` runtime stays strictly on the Web-standards surface (`fetch`, `WebSocket`, `EventSource`, `crypto.subtle` digest+HMAC, `URL`, `Blob`/`File`/`FormData`, `structuredClone`, `AbortController`, `performance.now`, `ReadableStream` body). Anything Node-specific (`Buffer`, `process.*`, `node:*`) or Bun-specific (`Bun.*`) is intentionally **not** polyfilled — see [ALTERNATIVES.md](ALTERNATIVES.md) for pure-JS libs that fill the common gaps, or switch to `full` for a full Node/Bun environment. See [RUNTIMES.md](RUNTIMES.md) for the parity table.

> ⚠️ **Architecture note — Electron:** Electron bundles its own Chromium build (~130 MB overhead) and exposes a full Node.js runtime in the main process. Tauri, Wails, and Tynd use the OS's native WebView (WebView2 / WKWebView / WebKitGTK). This explains both Electron's broader API surface and its larger binary footprint.

## Legend

| Symbol | Meaning |
|---|---|
| ✅ | Implemented and functional |
| ⚠️ | Partial / limited / requires community package |
| ❌ | Not implemented |
| 📱 | Mobile only (iOS/Android) |
| N/A | Not applicable to this framework's model |

---

## 1. Window — Core Operations

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Set title | ✅ | ✅ | ✅ | ✅ |
| Get current title | ❌ | ✅ | ❌ | ✅ |
| Set size | ✅ | ✅ | ✅ | ✅ |
| Get size (inner / outer) | ❌ | ✅ | ✅ | ✅ |
| Set position | ❌ | ✅ | ✅ | ✅ |
| Set relative position | ❌ | ❌ | ✅ | ❌ |
| Get position (inner / outer) | ❌ | ✅ | ✅ | ✅ |
| Center on screen | ✅ | ✅ | ✅ | ✅ |
| Set min / max size | ⚠️ config only | ✅ | ✅ | ✅ |
| Minimize / unminimize | ✅ | ✅ | ✅ | ✅ |
| Maximize / unmaximize | ✅ | ✅ | ✅ | ✅ |
| Toggle maximize | ❌ | ✅ | ✅ | ❌ |
| Set fullscreen | ✅ | ✅ | ✅ | ✅ |
| Simple fullscreen (macOS — no new Space) | ❌ | ✅ | ❌ | ✅ |
| Always on top | ✅ | ✅ | ✅ | ✅ |
| Always on bottom | ❌ | ✅ | ✅ | ✅ |
| Show / hide | ✅ | ✅ | ✅ | ✅ |
| Close / destroy | ❌ | ✅ | ✅ | ✅ |
| Focus | ✅ | ✅ | ✅ | ✅ |
| Request user attention / flash | ✅ | ✅ | ✅ | ✅ |
| Enable / disable | ❌ | ✅ | ✅ | ⚠️ |
| Set resizable | ❌ | ✅ | ✅ | ✅ |
| Set closable | ❌ | ✅ | ✅ | ✅ |
| Set maximizable | ❌ | ✅ | ✅ | ✅ |
| Set minimizable | ❌ | ✅ | ✅ | ✅ |
| Set movable | ❌ | ✅ | ❌ | ✅ |
| Set focusable | ❌ | ✅ | ❌ | ✅ |
| isMaximized / isMinimized / isFullscreen / isVisible | ✅ | ✅ | ✅ | ✅ |
| isDecorated / isResizable / isFocused | ❌ | ✅ | ⚠️ | ✅ |
| Skip taskbar | ❌ | ✅ | ✅ | ✅ |
| Content protection (screenshot block) | ❌ | ✅ | ✅ | ✅ |
| Show / hide menu bar | ❌ | ✅ | ✅ | ✅ |
| Set zoom factor / get zoom | ❌ | ✅ | ✅ | ✅ |
| Navigate to URL at runtime | ❌ | ✅ | ✅ | ✅ |
| Set HTML content at runtime | ❌ | ✅ | ✅ | ✅ |
| Reload | ❌ | ✅ | ✅ | ✅ |
| Visible on all workspaces | ❌ | ✅ | ❌ | ✅ |
| Prevent window overflowing monitor bounds | ❌ | ✅ | ❌ | ❌ |

---

## 2. Window — Appearance & Styling

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Frameless / borderless | ✅ | ✅ | ✅ | ✅ |
| Transparent background | ✅ | ✅ | ✅ | ✅ |
| Translucent window | ❌ | ❌ | ✅ | ✅ |
| CSS drag region | ✅ | ✅ | ✅ | ✅ |
| Set decorations at runtime | ✅ | ✅ | ✅ | ⚠️ |
| Set background color | ❌ | ✅ | ✅ | ✅ |
| Set shadow | ❌ | ✅ | ✅ | ✅ |
| Set window icon at runtime | ❌ | ✅ | ✅ | ✅ |
| Set theme (light / dark / system) | ❌ | ✅ | ✅ | ✅ |
| Set backdrop type at runtime | ❌ | ❌ | ✅ | ⚠️ |
| Mica / Acrylic / Tabbed (Windows 11) | ❌ | ✅ | ✅ | ✅ |
| LiquidGlass (macOS Sequoia) | ❌ | ❌ | ✅ | ❌ |
| Vibrancy / NSVisualEffect (macOS — 28 variants) | ❌ | ✅ | ✅ | ✅ |
| Titlebar style (overlay / hidden — macOS) | ❌ | ✅ | ✅ | ✅ |
| Hidden title text (macOS) | ❌ | ✅ | ✅ | ✅ |
| Traffic light position (macOS) | ❌ | ✅ | ❌ | ✅ |
| Window tabbing identifier (macOS) | ❌ | ✅ | ❌ | ❌ |
| Scrollbar style (Windows fluent overlay) | ❌ | ✅ | ❌ | ❌ |
| Taskbar progress bar (Windows / macOS) | ❌ | ✅ | ❌ | ✅ |
| Dock / taskbar badge count | ❌ | ✅ | ✅ | ✅ |
| Taskbar overlay icon (Windows) | ❌ | ✅ | ❌ | ✅ |
| Disable window icon (Windows) | ❌ | ❌ | ✅ | ❌ |
| Custom window shape (mask) | ❌ | ❌ | ✅ | ❌ |
| Reparent webview | ❌ | ✅ | ❌ | ❌ |
| Disable Aero shadow / rounded corners (Windows) | ❌ | ❌ | ✅ | ❌ |
| GPU acceleration disable | ❌ | ❌ | ✅ | ✅ |
| Set transparent at runtime | ❌ | ❌ | ✅ | ❌ |

---

## 3. Webview API

Tauri v2 has a distinct `Webview` class alongside `WebviewWindow`. Electron exposes `webContents` and `WebContentsView`.

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Webview / Window distinction | ❌ | ✅ | ❌ | ✅ |
| Get current webview | ❌ | ✅ | N/A | ✅ |
| Get all webviews | ❌ | ✅ | N/A | ✅ |
| Per-webview data directory | ❌ | ✅ | ✅ | ✅ |
| Per-webview data store identifier (macOS / iOS) | ❌ | ✅ | ❌ | ⚠️ |
| Incognito / private browsing webview | ❌ | ✅ | ❌ | ✅ |
| Background throttling policy | ❌ | ✅ | ❌ | ✅ |
| Accept first mouse (macOS) | ❌ | ✅ | ❌ | ❌ |
| Allow link preview (macOS / iOS) | ❌ | ✅ | ❌ | ❌ |
| Multiple webviews in one window | ❌ | ✅ | ❌ | ✅ |
| On navigation hook (intercept / cancel) | ❌ | ✅ | ❌ | ✅ |
| On page load callback | ❌ | ✅ | ❌ | ✅ |
| Disable input accessory view (iOS) | ❌ | ✅ 📱 | ❌ | ❌ |
| DevTools toggle per webview | ❌ | ✅ | ✅ | ✅ |
| Set user agent | ❌ | ✅ | ✅ | ✅ |
| Set zoom level / zoom factor | ❌ | ✅ | ✅ | ✅ |
| Capture page (screenshot of webview) | ❌ | ⚠️ | ❌ | ✅ |
| Navigation history — save / restore | ❌ | ❌ | ❌ | ✅ |
| Per-webview session | ❌ | ⚠️ | ❌ | ✅ |

---

## 4. Multi-Window

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Create secondary windows | ✅ | ✅ | ✅ | ✅ |
| Per-window independent configuration | ✅ | ✅ | ✅ | ✅ |
| List all open windows | ✅ | ✅ | ✅ | ✅ |
| Get window by label / ID | ✅ | ✅ | ✅ | ⚠️ |
| Close a specific window | ✅ | ✅ | ✅ | ✅ |
| Emit event to specific window | ⚠️ broadcast + label filter | ✅ | ✅ | ✅ |
| Modal window attached to parent | ❌ | ❌ | ✅ | ✅ |
| macOS Panel (overlay / NSPanel) | ❌ | ❌ | ✅ | ❌ |
| Set menu for specific window | ❌ | ✅ | ✅ | ✅ |

---

## 5. Window Events

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Close requested (preventable) | ✅ | ✅ | ✅ | ✅ |
| Resized | ✅ | ✅ | ✅ | ✅ |
| Moved | ✅ | ✅ | ✅ | ✅ |
| Focus gained / lost | ✅ | ✅ | ✅ | ✅ |
| Theme changed | ✅ | ✅ | ✅ | ✅ |
| DPI / scale changed | ✅ | ✅ | ✅ | ✅ |
| Minimized / maximized events | ✅ | ❌ | ✅ | ✅ |
| Fullscreen / unfullscreen events | ✅ | ❌ | ✅ | ✅ |
| Synchronous hooks with cancellation | ✅ `preventDefault()` | ❌ | ✅ | ✅ |
| App startup / shutdown hooks | ✅ `app.onReady` / `app.onClose` | ❌ | ✅ | ✅ |
| Hide on focus lost | ❌ | ❌ | ✅ | ❌ |
| Hide on Escape key | ❌ | ❌ | ✅ | ❌ |
| Visible on all workspaces (macOS) | ❌ | ✅ | ❌ | ✅ |
| Window-scoped event listeners | ❌ | ✅ | ❌ | ✅ |

---

## 6. Cursor & Mouse

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Set cursor icon | ❌ | ✅ | ❌ | ⚠️ |
| Set cursor position | ❌ | ✅ | ❌ | ❌ |
| Cursor grab | ❌ | ✅ | ❌ | ❌ |
| Cursor visible / hidden | ❌ | ✅ | ❌ | ❌ |
| Ignore cursor events (click-through) | ❌ | ✅ | ✅ | ✅ |
| Start drag programmatically | ❌ | ✅ | ❌ | ✅ |
| Start resize drag from edge | ❌ | ✅ | ❌ | ❌ |

---

## 7. Monitors & Screens

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| List all monitors | ❌ | ✅ | ✅ | ✅ |
| Get primary monitor | ❌ | ✅ | ✅ | ✅ |
| Get current window monitor | ❌ | ✅ | ✅ | ✅ |
| Get monitor from point | ❌ | ✅ | ❌ | ✅ |
| Get monitor by ID | ❌ | ❌ | ✅ | ⚠️ |
| Global cursor position | ❌ | ✅ | ❌ | ✅ |
| DPI / scale factor | ❌ | ✅ | ✅ | ✅ |
| Window positioner (13 preset positions) | ❌ | ✅ | ❌ | ❌ |

---

## 8. Drag & Drop

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Receive dropped files (native OS paths) | ❌ | ✅ | ✅ | ✅ |
| Targeted drop zones | ❌ | ❌ | ✅ | ❌ |
| Enable / disable file drop | ❌ | ❌ | ✅ | ❌ |
| HTML5 Drag & Drop (browser native) | ✅ | ✅ | ✅ | ✅ |

---

## 9. Dialogs

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Single file picker | ✅ | ✅ | ✅ | ✅ |
| Multiple file picker | ✅ | ✅ | ✅ | ✅ |
| Save file dialog | ✅ | ✅ | ✅ | ✅ |
| Directory picker | ❌ | ✅ | ✅ | ✅ |
| File type filters | ✅ | ✅ | ✅ | ✅ |
| Default path / filename | ✅ | ✅ | ✅ | ✅ |
| Message / alert (info) | ✅ | ✅ | ✅ | ✅ |
| OK/Cancel confirmation | ✅ | ✅ | ✅ | ✅ |
| Warning dialog | ❌ | ✅ | ✅ | ✅ |
| Error dialog | ❌ | ✅ | ✅ | ✅ |
| Custom button labels | ❌ | ✅ | ✅ | ✅ |
| Show hidden files | ❌ | ❌ | ✅ | ✅ |
| Can create directories | ❌ | ✅ | ✅ | ✅ |
| Treat packages as directories (macOS) | ❌ | ❌ | ✅ | ✅ |
| Attach dialog to window (modal) | ❌ | ❌ | ✅ | ✅ |
| Resolve macOS aliases | ❌ | ❌ | ✅ | ❌ |
| Native color picker | ❌ | ❌ | ❌ | ❌ |
| Native font picker | ❌ | ❌ | ❌ | ❌ |

---

## 10. System Tray

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Tray icon at startup | ✅ | ✅ | ✅ | ✅ |
| Multiple tray icons | ❌ | ✅ | ✅ | ✅ |
| Tooltip | ✅ | ✅ | ✅ | ✅ |
| Context menu | ✅ | ✅ | ✅ | ✅ |
| Left-click event | ✅ | ✅ | ✅ | ✅ |
| Right-click event | ✅ | ✅ | ✅ | ✅ |
| Double-click event | ✅ | ✅ | ✅ | ✅ |
| Mouse enter / move / leave | ❌ | ✅ | ✅ | ✅ |
| Menu item click | ✅ | ✅ | ✅ | ✅ |
| Dynamic icon update | ❌ | ✅ | ✅ | ✅ |
| Title text next to icon (macOS) | ❌ | ✅ | ✅ | ✅ |
| Template icon (macOS) | ❌ | ✅ | ✅ | ✅ |
| Dark mode icon variant | ❌ | ❌ | ✅ | ❌ |
| Show menu on left click | ❌ | ✅ | ❌ | ⚠️ |
| Show / hide tray | ❌ | ❌ | ✅ | ❌ |
| Attach window to tray icon | ❌ | ❌ | ✅ | ❌ |
| Position window near tray | ❌ | ✅ | ✅ | ❌ |
| Remove / destroy tray | ❌ | ✅ | ✅ | ✅ |
| Balloon tooltip (Windows) | ❌ | ❌ | ❌ | ✅ |
| Update menu item by ID at runtime | ❌ | ✅ | ✅ | ⚠️ |

---

## 11. Menu Bar — Application Menu

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Native menu bar | ✅ | ✅ | ✅ | ✅ |
| Set menu for specific window | ❌ | ✅ | ✅ | ✅ |
| Recursive submenus | ✅ | ✅ | ✅ | ✅ |
| Separators | ✅ | ✅ | ✅ | ✅ |
| Action items with custom ID | ✅ | ✅ | ✅ | ✅ |
| Enabled / disabled items | ✅ | ✅ | ✅ | ✅ |
| Native roles (quit / copy / paste / undo…) | ✅ | ✅ | ✅ | ✅ |
| Menu item click event | ✅ | ✅ | ✅ | ✅ |
| Keyboard accelerators | ❌ | ✅ | ✅ | ✅ |
| Checkbox items | ❌ | ✅ | ✅ | ✅ |
| Radio items | ❌ | ❌ | ✅ | ✅ |
| Icon in menu item | ❌ | ✅ | ✅ | ✅ |
| Icon in submenu | ❌ | ✅ | ✅ | ❌ |
| Update submenu text at runtime | ❌ | ✅ | ✅ | ✅ |
| Tooltip on item | ❌ | ❌ | ✅ | ❌ |
| Dynamic add / remove items at runtime | ❌ | ✅ | ✅ | ⚠️ |
| Toggle / hide menu bar | ❌ | ✅ | ✅ | ✅ |
| Clone a menu | ❌ | ❌ | ✅ | ❌ |
| macOS Windows menu (manage open windows) | ❌ | ❌ | ✅ | ❌ |

---

## 12. Context Menu

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Show context menu programmatically | ❌ | ✅ | ✅ | ✅ |
| Context data on click | ❌ | ❌ | ✅ | ❌ |
| Named context menus | ❌ | ❌ | ✅ | ⚠️ |
| Default browser context menu enabled | ❌ | ❌ | ✅ | ✅ |

---

## 13. Clipboard

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Read text | ✅ | ✅ | ✅ | ✅ |
| Write text | ✅ | ✅ | ✅ | ✅ |
| Read image | ❌ | ✅ | ❌ | ✅ |
| Write image | ❌ | ✅ | ❌ | ✅ |
| Read / write HTML | ❌ | ✅ | ❌ | ✅ |
| Clear clipboard | ❌ | ✅ | ❌ | ✅ |
| Clipboard change monitoring | ❌ | ✅ | ❌ | ⚠️ |
| Read / write file paths (drag sources) | ❌ | ✅ | ❌ | ✅ |
| Read / write custom buffer formats | ❌ | ⚠️ | ❌ | ✅ |

---

## 14. Notifications

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Basic notification (title + body) | ✅ | ✅ | ✅ | ✅ |
| Custom icon | ❌ | ✅ | ❌ | ✅ |
| Check permission | ❌ | ✅ | N/A | ✅ |
| Request permission | ❌ | ✅ | N/A | ❌ |
| Action buttons | ❌ | ✅ | ✅ | ✅ |
| Interactive text input | ❌ | ✅ | ✅ | ❌ |
| Sound | ❌ | ✅ | ❌ | ✅ |
| Scheduled (delayed / recurring) | ❌ | ✅ | ❌ | ❌ |
| Notification groups | ❌ | ✅ | ❌ | ❌ |
| Cancel / remove notifications | ❌ | ✅ | ❌ | ✅ |
| Notification channels (Android) | ❌ | ✅ 📱 | N/A | N/A |
| On notification received callback | ❌ | ✅ | ✅ | ✅ |
| On action callback | ❌ | ✅ | ✅ | ✅ |

---

## 15. Global Shortcuts

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Register global hotkey | ❌ | ✅ | ✅ | ✅ |
| Unregister hotkey | ❌ | ✅ | ✅ | ✅ |
| Check if registered | ❌ | ✅ | ❌ | ✅ |
| Callback on trigger | ❌ | ✅ | ✅ | ✅ |
| Key state (pressed / released) | ❌ | ✅ | ❌ | ❌ |
| Wayland support | ❌ | ❌ | ❌ | ❌ |

---

## 16. Shell & File System

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Open URL in default browser | ✅ | ✅ | ✅ | ✅ |
| Open file / folder with default app | ✅ | ✅ | ✅ | ✅ |
| Reveal in file manager | ❌ | ✅ | ✅ | ✅ |
| Execute shell command | ✅ | ✅ | ⚠️ | ✅ |
| Execute bundled sidecar binary | ✅ | ✅ | ⚠️ | ✅ |
| Capture stdout / stderr | ✅ | ✅ | ⚠️ | ✅ |
| Kill a spawned process | ✅ | ✅ | ❌ | ✅ |
| Interactive PTY terminal in-app | ✅ | ⚠️ | ❌ | ✅ |
| FS: read file (text / binary) | ✅ | ✅ | ⚠️ | ✅ |
| FS: write file (text / binary) | ✅ | ✅ | ⚠️ | ✅ |
| FS: create / remove directory | ✅ | ✅ | ⚠️ | ✅ |
| FS: remove / rename file | ✅ | ✅ | ⚠️ | ✅ |
| FS: list directory | ✅ | ✅ | ⚠️ | ✅ |
| FS: file metadata | ✅ | ✅ | ⚠️ | ✅ |
| FS: copy file / directory | ✅ | ✅ | ⚠️ | ✅ |
| FS: file watcher | ❌ | ✅ | ❌ | ✅ |
| FS: file handle (seek / partial read) | ❌ | ✅ | ❌ | ✅ |
| Scoped file access (allow / deny patterns) | ❌ | ✅ | ❌ | ❌ |
| FS: trash / move to recycle bin | ❌ | ⚠️ | ❌ | ✅ |
| FS: symbolic link create / read | ❌ | ✅ | ⚠️ | ✅ |
| FS: hard link | ❌ | ✅ | ⚠️ | ✅ |

---

## 17. IPC & Events

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Typed RPC frontend -> backend | ✅ | ✅ | ✅ | ✅ |
| Auto-generated TS types (zero build step) | ✅ | ⚠️ | ✅ | ⚠️ |
| Backend -> frontend events | ✅ | ✅ | ✅ | ✅ |
| Frontend -> backend events | ❌ | ✅ | ✅ | ✅ |
| `once()` listener | ✅ | ✅ | ✅ | ✅ |
| Listen N times | ❌ | ❌ | ✅ | ❌ |
| Window-scoped event listeners | ❌ | ✅ | ❌ | ✅ |
| Plugin event listeners | ❌ | ✅ | N/A | N/A |
| Streaming channels | ❌ | ✅ | ❌ | ⚠️ |
| Framework detection at runtime | ❌ | ✅ | ❌ | ✅ |
| Eval JS from backend / main process | ❌ | ✅ | ✅ | ✅ |
| Zero-network IPC (no HTTP / WS) | ✅ | ✅ | ❌ | ✅ |
| Frontend asset serving via custom scheme | ✅ | ✅ | ❌ | ✅ |
| Firewall prompt | ❌ | ❌ | ✅ | ❌ |
| Invoke timeout | ❌ | ❌ | ✅ | ❌ |
| Call cancellation | ❌ | ❌ | ✅ | ❌ |
| Emit to specific window | ❌ | ✅ | ✅ | ✅ |
| Custom IPC transport | ❌ | ❌ | ✅ | ✅ |
| HTTP asset server middleware | ❌ | ⚠️ | ✅ | ✅ |
| Server mode (no GUI) | ❌ | ❌ | ✅ | ✅ |
| Serve local files via protocol / custom scheme | ❌ | ✅ | ❌ | ✅ |
| MessagePort / MessageChannel (transferable) | ❌ | ❌ | ❌ | ✅ |
| `postMessage` with transferable objects | ❌ | ❌ | ❌ | ✅ |

---

## 18. HTTP Client & WebSocket

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| HTTP requests from frontend | ✅ | ✅ | ⚠️ | ✅ |
| WebSocket client | ✅ | ✅ | ❌ | ✅ |
| File upload with progress | ✅ | ✅ | ❌ | ✅ |
| File download with progress | ✅ | ✅ | ❌ | ✅ |

---

## 19. Auto-Updater

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Check for updates | ❌ | ✅ | ❌ | ✅ |
| Download + install | ❌ | ✅ | ❌ | ✅ |
| Progress callback | ❌ | ✅ | ❌ | ✅ |
| Periodic auto-check | ❌ | ✅ | ❌ | ✅ |
| GitHub Releases support | ❌ | ✅ | ❌ | ✅ |
| Signature verification | ❌ | ✅ | ❌ | ✅ |
| Delta updates (binary diff) | ❌ | ✅ | ❌ | ✅ |
| Custom update server | ❌ | ✅ | ❌ | ✅ |
| Proxy / custom headers | ❌ | ✅ | ❌ | ✅ |
| Allow downgrades | ❌ | ✅ | ❌ | ✅ |

---

## 20. Single Instance & Deep Linking

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Single instance lock | ✅ | ✅ | ✅ | ✅ |
| Focus existing window on 2nd launch | ✅ | ✅ | ✅ | ✅ |
| Pass argv / cwd to existing process | ✅ | ✅ | ✅ | ✅ |
| Encrypted inter-instance comms | ❌ | ❌ | ✅ | ❌ |
| Custom URL scheme (`myapp://`) | ✅ | ✅ | ❌ | ✅ |
| Scheme registered at build time | ✅ | ✅ | ❌ | ✅ |
| `onOpenUrl` handler | ✅ | ✅ | ❌ | ✅ |
| File type associations (`.ext -> app`) | ❌ | ❌ | ✅ | ✅ |

---

## 21. Autolaunch

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Enable launch at system startup | ❌ | ✅ | ❌ | ✅ |
| Disable autolaunch | ❌ | ✅ | ❌ | ✅ |
| Check autolaunch status | ❌ | ✅ | ❌ | ✅ |

---

## 22. Persistent Storage

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Key-value store — get / set / delete | ✅ | ✅ | ✅ | ⚠️ |
| Store — has / keys / values / entries / length | ✅ | ✅ | ❌ | ⚠️ |
| Store — clear / reset / reload | ✅ | ✅ | ❌ | ⚠️ |
| Store — auto-save + events on change | ⚠️ | ✅ | ❌ | ⚠️ |
| SQLite / relational DB | ✅ | ✅ | ✅ | ⚠️ |
| Encrypted secure storage | ❌ | ✅ | ❌ | ✅ |
| Persisted scope (runtime permission changes saved) | ❌ | ✅ | N/A | N/A |
| Cookies API (read / set / delete) | ❌ | ⚠️ | ❌ | ✅ |
| HTTP cache control (size, clear) | ❌ | ❌ | ❌ | ✅ |
| Proxy configuration | ❌ | ⚠️ | ❌ | ✅ |

---

## 23. Logging

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Structured logging (debug / info / warn / error) | ❌ | ✅ | ✅ | ⚠️ |
| Write to file with rotation | ❌ | ✅ | ✅ | ⚠️ |
| JS -> native bridge (console.log captured) | ❌ | ✅ | ❌ | ✅ |
| Multiple log targets (file / stdout / Sentry…) | ❌ | ✅ | ✅ | ⚠️ |
| Log level per environment (dev / prod) | ❌ | ✅ | ✅ | ⚠️ |

---

## 24. App-Level APIs

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Get app name | ❌ | ✅ | ✅ | ✅ |
| Get app version | ❌ | ✅ | ✅ | ✅ |
| Get framework version | ❌ | ✅ | N/A | ✅ |
| Get bundle identifier | ❌ | ✅ | ❌ | ⚠️ |
| Get bundle type | ❌ | ✅ | ❌ | ❌ |
| Get default window icon | ❌ | ✅ | ❌ | ❌ |
| Show / hide app (macOS dock) | ❌ | ✅ | ✅ | ✅ |
| Set dock visibility (macOS) | ❌ | ✅ | ✅ | ✅ |
| App quits after last window closed | ❌ | ❌ | ✅ | ✅ |
| About / info panel | ❌ | ❌ | ✅ | ✅ |
| exit(code) | ❌ | ✅ | ✅ | ✅ |
| relaunch() | ❌ | ✅ | ❌ | ✅ |
| Back button (Android) | ❌ | ✅ 📱 | N/A | N/A |
| Data store management (Apple platforms) | ❌ | ✅ 📱 | N/A | ❌ |

---

## 25. OS & Environment

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Platform name | ❌ | ✅ | ✅ | ✅ |
| CPU architecture | ❌ | ✅ | ✅ | ✅ |
| OS version | ❌ | ✅ | ✅ | ✅ |
| Hostname | ❌ | ✅ | ❌ | ✅ |
| Locale (BCP-47) | ❌ | ✅ | ❌ | ✅ |
| EOL character | ❌ | ✅ | ❌ | ✅ |
| OS family (unix / windows) | ❌ | ✅ | ❌ | ✅ |
| Dark mode detection | ❌ | ✅ | ✅ | ✅ |
| System accent color | ❌ | ❌ | ✅ | ✅ |
| CLI argument parsing | ❌ | ✅ | ❌ | ✅ |
| Print page | ❌ | ❌ | ✅ | ✅ |
| Snap Assist (Windows 11) | ❌ | ❌ | ✅ | ❌ |

---

## 26. Path Utilities

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| App data / local data dir | ✅ | ✅ | ❌ | ✅ |
| App config / cache / log dir | ✅ | ✅ | ❌ | ✅ |
| App resource dir | ❌ | ✅ | ❌ | ✅ |
| Home / temp dir | ✅ | ✅ | ❌ | ✅ |
| Desktop / Downloads / Documents | ❌ | ✅ | ❌ | ✅ |
| Pictures / Music / Video | ❌ | ✅ | ❌ | ✅ |
| Font dir (macOS) | ❌ | ✅ | ❌ | ❌ |
| Public / runtime dir | ❌ | ✅ | ❌ | ❌ |
| Executable path | ✅ | ✅ | ❌ | ✅ |
| System-level cache / config / data dirs | ✅ | ✅ | ❌ | ⚠️ |
| Path manipulation (join / normalize / resolve…) | ✅ | ✅ | ❌ | ✅ |
| XDG-compliant on Linux | ❌ | ✅ | N/A | ✅ |
| Serve local file via custom scheme / protocol | ❌ | ✅ | ❌ | ✅ |

---

## 27. Security & Permissions

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Capability-based ACL | ❌ | ✅ | ❌ | ❌ |
| Default-deny model | ❌ | ✅ | ❌ | ❌ |
| Command allowlist / denylist | ❌ | ✅ | ❌ | ❌ |
| Scoped FS access (path patterns) | ❌ | ✅ | ❌ | ❌ |
| Scoped HTTP access (URL patterns) | ❌ | ✅ | ❌ | ❌ |
| Auto-injected CSP | ❌ | ✅ | ❌ | ❌ |
| Permission request handler | ❌ | ✅ | ❌ | ✅ |
| Biometric authentication | ❌ | ✅ 📱 | ❌ | ❌ |
| Structural security (export = exposure surface) | ✅ | ❌ | ❌ | ❌ |
| Context isolation (renderer ↔ preload boundary) | ❌ | ✅ | ❌ | ✅ |
| Preload scripts | ❌ | ⚠️ | ❌ | ✅ |
| contextBridge — expose selective APIs to renderer | ❌ | ❌ | ❌ | ✅ |
| Renderer sandbox mode | ❌ | ❌ | ❌ | ✅ |

---

## 28. Mobile (iOS / Android)

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| iOS support | ❌ | ✅ | ❌ | ❌ |
| Android support | ❌ | ✅ | ❌ | ❌ |
| Barcode / QR scanner | ❌ | ✅ 📱 | ❌ | ❌ |
| NFC (read / write tags) | ❌ | ✅ 📱 | ❌ | ❌ |
| Haptics / vibration | ❌ | ✅ 📱 | ❌ | ❌ |
| Geolocation | ❌ | ✅ 📱 | ❌ | ❌ |
| Share (files / text) | ❌ | ✅ 📱 | ❌ | ❌ |
| Open in-app browser | ❌ | ✅ 📱 | ❌ | ❌ |

---

## 29. Build & Distribution

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| **Windows** | | | | |
| NSIS installer (.exe) | ✅ | ✅ | ✅ | ✅ |
| MSI / WiX (.msi) | ✅ WiX v3 | ✅ | ❌ | ✅ |
| WebView2 bootstrapper embedded | ❌ | ✅ | ❌ | N/A |
| WebView2 custom user data path | ❌ | ✅ | ✅ | N/A |
| Code signing | ❌ | ✅ | ✅ | ✅ |
| **macOS** | | | | |
| .app bundle | ✅ | ✅ | ✅ | ✅ |
| .dmg disk image | ✅ | ✅ | ✅ | ✅ |
| codesign | ❌ | ✅ | ✅ | ✅ |
| Notarization | ❌ | ✅ | ❌ | ✅ |
| **Linux** | | | | |
| .deb (Debian / Ubuntu) | ✅ | ✅ | ❌ | ✅ |
| .AppImage | ✅ | ✅ | ✅ | ✅ |
| .rpm (Fedora / RHEL) | ✅ needs rpmbuild | ✅ | ❌ | ✅ |
| .desktop file generation | ✅ | ✅ | ✅ | ✅ |
| GTK3 / WebKitGTK 4.1 | ✅ | ✅ | ✅ | N/A |
| GTK4 / WebKitGTK 6.0 (experimental) | ❌ | ❌ | ✅ | N/A |
| Wayland — fractional scaling + NVIDIA DMA-BUF fix | ❌ | ❌ | ✅ | ✅ |
| Snap package (.snap) | ❌ | ❌ | ❌ | ✅ |
| Flatpak (.flatpak) | ❌ | ⚠️ | ❌ | ✅ |
| Microsoft Store (MSIX / APPX) | ❌ | ❌ | ❌ | ✅ |
| **General** | | | | |
| Build hooks (before / after) | ❌ | ✅ | ✅ | ✅ |
| Icon embedded in binary | ✅ | ✅ | ✅ | ✅ |
| Icon generation (SVG/PNG -> multi-size ICO / ICNS / hicolor PNG set) | ✅ | ✅ | ✅ | ⚠️ |
| Cross-compilation | ❌ | ✅ | ✅ | ✅ |
| PE patch (suppress Windows console) | ✅ | ✅ | ✅ | ✅ |
| Build tools auto-downloaded | ✅ NSIS/WiX/appimagetool | ❌ | ❌ | ✅ |
| Plugin / service scaffold CLI | ❌ | ✅ | ✅ | ❌ |

---

## 30. Developer Experience & CLI

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Project scaffold | ✅ | ✅ | ✅ | ✅ |
| Templates (vanilla / React / Vue…) | ✅ | ✅ | ✅ | ✅ |
| Runtime selector (full vs lite) | ✅ | N/A | N/A | N/A |
| Dev mode with HMR proxy | ✅ | ✅ | ✅ | ⚠️ |
| Hot reload backend | ✅ | ❌ | ✅ | ⚠️ |
| Start command (run prebuilt, no HMR) | ✅ | ❌ | ❌ | ⚠️ |
| Production build command | ✅ | ✅ | ✅ | ✅ |
| Environment diagnostics | ✅ | ✅ | ✅ | ⚠️ |
| Zero-codegen TS bindings | ✅ | ⚠️ | ✅ | ⚠️ |
| Generate desktop / syso files | ❌ | ❌ | ✅ | N/A |
| Generate AppImage | ✅ | ❌ | ✅ | ✅ |
| Task runner integration | ❌ | ❌ | ✅ | ❌ |
| Service scaffold | ❌ | ❌ | ✅ | ❌ |
| Plugin scaffold + add / remove | ❌ | ✅ | ❌ | ❌ |
| TypeScript backend (no extra language) | ✅ | ❌ | ❌ | ✅ |
| Embedded JS runtime option | ✅ | N/A | N/A | N/A |
| Official plugin ecosystem | ❌ | ✅ | ✅ | ✅ |
| Plugin / service development framework | ❌ | ✅ | ✅ | ❌ |
| Built-in unit-test runner | ✅ `bun test` + `cargo test` | ❌ | ❌ | ❌ |
| Framework-level test suite | ⚠️ core + cache + pack + OS APIs | ❌ | ⚠️ | ⚠️ |

---

## 31. IPC Architecture — Technical Detail

| Mechanism | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| JS -> native transport | ✅ wry `postMessage` | ✅ wry `postMessage` | ❌ HTTP POST | ✅ Chromium IPC |
| Native -> JS push | ✅ `evaluate_script` | ✅ `evaluate_script` | ❌ WebSocket | ✅ `webContents.send` |
| Frontend asset serving | ✅ custom scheme | ✅ custom scheme | ❌ HTTP server | ✅ loadFile / custom scheme |
| Opens TCP port | ❌ | ❌ | ✅ | ❌ |
| Firewall prompt possible | ❌ | ❌ | ✅ | ❌ |
| Shared native crates with Tauri | ✅ | — | ❌ | ❌ |

---

## 32. Printing & PDF Export

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Programmatic print (open system dialog) | ❌ | ❌ | ✅ | ✅ |
| Silent print (no dialog) | ❌ | ❌ | ❌ | ✅ |
| Page range / copies / duplex options | ❌ | ❌ | ❌ | ✅ |
| Choose printer by name | ❌ | ❌ | ❌ | ✅ |
| Print preview | ❌ | ❌ | ❌ | ✅ |
| `printToPDF` (webview -> PDF buffer) | ❌ | ❌ | ❌ | ✅ |
| List available printers | ❌ | ❌ | ❌ | ✅ |

---

## 33. Screen & Media Capture

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Screen / window source enumeration | ❌ | ❌ | ❌ | ✅ |
| `getDisplayMedia` / MediaStream | ❌ | ❌ | ❌ | ✅ |
| macOS 15 system picker | ❌ | ❌ | ❌ | ✅ |
| Thumbnail previews for source picker | ❌ | ❌ | ❌ | ✅ |
| Capture own window (screenshot) | ❌ | ⚠️ | ❌ | ✅ |
| Camera / microphone permission prompt | ❌ | ❌ | ❌ | ✅ |
| WebRTC screen sharing | ❌ | ❌ | ❌ | ✅ |

---

## 34. Accessibility

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Enable / disable accessibility support | ❌ | ❌ | ❌ | ✅ |
| VoiceOver (macOS) integration | ⚠️ | ⚠️ | ⚠️ | ✅ |
| Narrator (Windows) integration | ⚠️ | ⚠️ | ⚠️ | ✅ |
| ARIA attribute forwarding to AT | ✅ | ✅ | ✅ | ✅ |
| Reduced motion / high contrast detection | ❌ | ❌ | ❌ | ✅ |
| System font-size override awareness | ❌ | ❌ | ❌ | ✅ |

---

## 35. Spellcheck & Find in Page

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Built-in spellchecker | ❌ | ❌ | ❌ | ✅ |
| Custom dictionary | ❌ | ❌ | ❌ | ✅ |
| Language list / switch | ❌ | ❌ | ❌ | ✅ |
| Context menu spelling suggestions | ❌ | ❌ | ❌ | ✅ |
| `findInPage` (Cmd+F style) | ❌ | ❌ | ❌ | ✅ |
| `stopFindInPage` | ❌ | ❌ | ❌ | ✅ |

---

## 36. Extensions, WebFrame & Service Workers

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Load Chrome extension | ❌ | ❌ | ❌ | ✅ |
| Extension manifest v3 support | ❌ | ❌ | ❌ | ✅ |
| `webFrame` / `webFrameMain` API | ❌ | ❌ | ❌ | ✅ |
| Service worker — register from main | ❌ | ❌ | ❌ | ✅ |
| Service worker preload scripts | ❌ | ❌ | ❌ | ✅ |
| SharedArrayBuffer / COOP-COEP control | ❌ | ⚠️ | ❌ | ✅ |

---

## 37. In-app Purchase (Storefronts)

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| macOS StoreKit — product lookup | ❌ | ❌ | ❌ | ✅ |
| macOS StoreKit — purchase / restore | ❌ | ❌ | ❌ | ✅ |
| Windows Store — Microsoft.Services.Store | ❌ | ❌ | ❌ | ❌ |
| Receipt validation | ❌ | ❌ | ❌ | ✅ |
| Subscription status | ❌ | ❌ | ❌ | ✅ |

---

## 38. Electron-Specific APIs

Features available in Electron with no direct equivalent in the other frameworks.

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Screen / window capture (desktopCapturer) | ❌ | ❌ | ❌ | ✅ |
| Power monitor (sleep / wake / idle) | ❌ | ❌ | ❌ | ✅ |
| Prevent display / app sleep (powerSaveBlocker) | ❌ | ❌ | ❌ | ✅ |
| Crash reporting | ✅ | ❌ | ❌ | ✅ |
| Touch Bar (macOS) | ❌ | ❌ | ❌ | ✅ |
| Push notifications (macOS APNs) | ❌ | ❌ | ❌ | ✅ |
| Session management (cookies / cache / proxy) | ❌ | ❌ | ❌ | ✅ |
| Network request interceptor | ❌ | ❌ | ❌ | ✅ |
| Custom protocol registration | ❌ | ❌ | ❌ | ✅ |
| System preferences API (macOS / Windows) | ❌ | ❌ | ❌ | ✅ |
| Service Worker support | ❌ | ❌ | ❌ | ✅ |
| Full Node.js stdlib in main process | ❌ | ❌ | ❌ | ✅ |
| Bundled Chromium (no OS WebView dependency) | ❌ | ❌ | ❌ | ✅ |

---

## 39. Concurrency & Native Compute

| Feature | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Unified `workers` API (spawn JS on OS thread) | ✅ | ❌ | ❌ | ⚠️ |
| Streaming RPC (AsyncGenerator across IPC, typed frontend handle) | ✅ | ❌ | ❌ | ⚠️ |
| Worker pool (frontend-invokable) | ✅ | ⚠️ | ❌ | ⚠️ |
| Native hash helpers (blake3 / sha256 / sha384 / sha512) | ✅ | ⚠️ | ⚠️ | ⚠️ |
| OS calls run off JS event loop | ✅ | ✅ | ✅ | ✅ |
| Long-running Rust ops don't block JS | ✅ | ✅ | ⚠️ | N/A |
| `SharedArrayBuffer` / `Atomics` in backend | ⚠️ | ⚠️ | ❌ | ✅ |
| Streaming PTY on its own thread | ✅ | ⚠️ | ❌ | ✅ |

---

## Summary Score

| Category | Tynd | Tauri v2 | Wails v3 | Electron |
|---|---|---|---|---|
| Window — core ops | 16/39 | 38/39 | 29/39 | 37/39 |
| Window — appearance | 4/27 | 19/27 | 18/27 | 18/27 |
| Webview API | 0/19 | 17/19 | 2/19 | 16/19 |
| Multi-window | 5/9 | 8/9 | 7/9 | 8/9 |
| Window events | 10/14 | 11/14 | 12/14 | 13/14 |
| Cursor & mouse | 0/7 | 7/7 | 2/7 | 3/7 |
| Monitors & screens | 0/8 | 7/8 | 6/8 | 7/8 |
| Drag & drop | 1/4 | 3/4 | 4/4 | 3/4 |
| Dialogs | 7/18 | 13/18 | 14/18 | 14/18 |
| System tray | 7/20 | 15/20 | 18/20 | 14/20 |
| Menu bar (app) | 6/19 | 15/19 | 15/19 | 13/19 |
| Context menu | 0/4 | 3/4 | 4/4 | 3/4 |
| Clipboard | 2/9 | 8/9 | 2/9 | 8/9 |
| Notifications | 1/13 | 13/13 | 6/13 | 9/13 |
| Global shortcuts | 0/6 | 6/6 | 4/6 | 4/6 |
| Shell & FS | 2/21 | 20/21 | 6/21 | 20/21 |
| IPC & Events | 6/23 | 17/23 | 11/23 | 17/23 |
| HTTP & WebSocket | 4/4 | 4/4 | 1/4 | 4/4 |
| Auto-updater | 0/10 | 10/10 | 0/10 | 10/10 |
| Single instance & deep linking | 6/8 | 7/8 | 4/8 | 7/8 |
| Autolaunch | 0/3 | 3/3 | 0/3 | 3/3 |
| Persistent storage | 4/10 | 7/10 | 3/10 | 7/10 |
| Logging | 0/5 | 5/5 | 4/5 | 2/5 |
| App-level APIs | 0/14 | 11/14 | 7/14 | 11/14 |
| OS & Environment | 0/12 | 11/12 | 5/12 | 11/12 |
| Path utilities | 0/13 | 13/13 | 0/13 | 11/13 |
| Security & permissions | 1/13 | 8/13 | 0/13 | 6/13 |
| Mobile | 0/8 | 8/8 | 0/8 | 0/8 |
| Build & distribution | 3/26 | 19/26 | 13/26 | 21/26 |
| DX & CLI | 7/17 | 12/17 | 11/17 | 9/17 |
| Printing & PDF | 0/7 | 0/7 | 1/7 | 7/7 |
| Screen & media capture | 0/7 | 0/7 | 0/7 | 7/7 |
| Accessibility | 1/6 | 1/6 | 1/6 | 6/6 |
| Spellcheck & find | 0/6 | 0/6 | 0/6 | 6/6 |
| Extensions / WebFrame / SW | 0/6 | 0/6 | 0/6 | 6/6 |
| In-app purchase | 0/5 | 0/5 | 0/5 | 4/5 |
| Electron-specific APIs | 0/13 | 0/13 | 0/13 | 13/13 |
| **Total** | **~90/503 (18%)** | **~368/503 (73%)** | **~189/503 (38%)** | **~357/503 (71%)** |

> **Note on scores:** Tynd is early-stage — the foundations (wry + tao IPC, zero-codegen typed RPC, dual runtimes) are solid. Electron's score benefits from Node.js stdlib covering FS, shell, path, and OS utilities natively, plus Chromium-native features like printing, spellcheck, screen capture, and extensions. Tauri v2's breadth is driven by its 31 official plugins and mobile platform support.

---

## Tynd unique strengths

| Feature | Description |
|---|---|
| **Zero-codegen typed RPC** | `createBackend<typeof backend>()` — types come from `typeof`, no generated files, no build step |
| **100% TypeScript backend** | No Rust or Go to learn or maintain |
| **Dual runtimes** | `full` (Bun subprocess — full npm ecosystem, Bun JIT) or `lite` (embedded JS engine, smaller binary, faster cold start) |
| **Direct OS APIs from frontend** | `dialog`, `tyndWindow`, `clipboard`, `shell`, `notification`, `tray` call into Rust directly — no round-trip through the TypeScript backend |
| **Same IPC stack as Tauri v2** | wry + tao + `tynd://` custom scheme — zero TCP, zero WebSocket, no firewall prompt |
| **Zero-copy binary IPC** | dedicated `tynd-bin://` scheme for multi-MB payloads — no base64, `ArrayBuffer` end-to-end. 5-10x faster than JSON-encoded binary |
| **Structural security model** | The exposure surface is the exported module — code and security policy cannot drift apart |

## Tauri v2 unique strengths

| Feature | Description |
|---|---|
| **Capability-based ACL** | Fine-grained permissions per command, file path, URL — default-deny model |
| **31 official plugins** | Stronghold, biometric, NFC, SQL, HTTP, WebSocket, autostart, deep-link, updater, store, persisted-scope… |
| **Mobile support** | iOS + Android with dedicated plugins (barcode, NFC, haptics, geolocation, share) |
| **28 window visual effects** | All Mica/Acrylic/Tabbed variants + 28 NSVisualEffect macOS variants |
| **59 native menu icons** | NSImage system symbols in menu items (macOS) |
| **Separate Webview API** | Distinct Webview / WebviewWindow classes — incognito, data store isolation, background throttling policy |
| **Full cursor management** | Grab, icon, position, visibility, click-through |
| **Sidecar binaries** | Bundle and run native executables alongside the app |
| **WebView2 bootstrapper** | Embedded installer for Windows machines without WebView2 |
| **Delta updates** | Binary diff for minimal update download size |
| **Multi-webview** | Multiple independent webviews inside a single native window (stabilised in Tauri 2.x) |
| **Navigation interception** | Rust-side hook to validate or block URLs before load |

## Wails v3 unique strengths

| Feature | Description |
|---|---|
| **LiquidGlass** | macOS Sequoia native glass effect |
| **SnapAssist** | Windows 11 snap layout trigger |
| **Custom window shape (mask)** | Non-rectangular windows on Windows |
| **54 menu roles** | Full set of standard macOS/Windows menu actions |
| **File type associations** | Register as the handler for specific file extensions |
| **Synchronous event hooks** | Cancellable hooks on system events |
| **Custom IPC transport** | Replace the default HTTP/WS mechanism entirely |
| **Encrypted single instance** | AES-256-GCM communication between instances |
| **GTK4 / WebKitGTK 6.0** | Experimental support via `-tags gtk4` |
| **HTTP middleware + Gin services** | Custom `http.Handler` and full Gin router integration on the asset server |
| **macOS WebviewPanel** | Overlay / NSPanel-style windows |
| **SQLite service (example)** | JS bindings, prepared statements, cancellable queries — shipped as an example service in v3 alpha |
| **Server mode** | Run the app without a GUI window, serving over HTTP/WS only |
| **Wayland improvements** | Fractional scaling support, auto-disable DMA-BUF on NVIDIA |
| **HideOnFocusLost / HideOnEscape** | Built-in auto-hide behaviors |

## Electron unique strengths

| Feature | Description |
|---|---|
| **Bundled Chromium** | Consistent rendering cross-platform — no OS WebView version surprises |
| **Full Node.js stdlib** | `fs`, `path`, `child_process`, `crypto`, `http`, `os`, `stream`… all available without IPC |
| **desktopCapturer** | Screen and window capture API — thumbnail + MediaStream |
| **powerMonitor** | System power events: suspend, resume, lock-screen, idle detection |
| **powerSaveBlocker** | Prevent display or app sleep programmatically |
| **crashReporter** | Built-in crash dump collection and upload |
| **Touch Bar (macOS)** | Full Touch Bar API with buttons, sliders, popovers, scrubbers |
| **session API** | Cookies, cache, proxy config, custom headers, per-partition isolation |
| **protocol module** | Intercept and handle any URL scheme with full request/response control |
| **contextBridge + preload** | Safely expose selective APIs to the renderer — battle-tested sandboxing pattern |
| **Service Workers** | Native Chromium Service Worker support for offline-capable apps |
| **safeStorage** | OS-level encryption (Keychain / DPAPI / libsecret) for secrets |
| **Massive npm ecosystem** | Any npm package works in the main process — no plugin porting needed |
| **Printing + printToPDF** | Silent print, page ranges, printer selection, `webContents.printToPDF` |
| **Built-in spellchecker** | Custom dictionaries, language switching, context-menu suggestions |
| **Find in page** | `findInPage` / `stopFindInPage` with match highlighting |
| **Chrome extensions** | Load unpacked extensions via `session.loadExtension` (MV3) |
| **StoreKit (inAppPurchase)** | macOS product lookup, purchase, restore, receipt validation |
| **MessageChannelMain** | Transferable-object IPC ports between main and renderers |
