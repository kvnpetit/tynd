# bunview vs Tauri v2 vs Wails v3 — Complete Feature Comparison

> Last updated: April 2026 | Desktop only (mobile excluded)

## Legend

| Symbol | Meaning |
|---|---|
| ✅ | Implemented and functional |
| ⚠️ | Partial / limited |
| ❌ | Not implemented |
| N/A | Not applicable |

---

## 1. Window Management

### 1.1 Core Window Operations

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| `setTitle()` | ✅ | ✅ | ✅ |
| `setSize()` / `getSize()` | ✅ | ✅ (`innerSize`, `outerSize`) | ✅ (`Size`, `Width`, `Height`) |
| `setPosition()` / `getPosition()` | ✅ | ✅ (`innerPosition`, `outerPosition`) | ✅ (`Position`, `RelativePosition`) |
| `setMinSize()` / `setMaxSize()` | ✅ | ✅ `setSizeConstraints()` | ✅ |
| `center()` | ✅ | ✅ | ✅ |
| `minimize()` / `maximize()` / `restore()` | ✅ | ✅ + `unmaximize()`, `unminimize()`, `toggleMaximize()` | ✅ + `ToggleMaximise()`, `UnMaximise()`, `UnMinimise()` |
| `fullscreen()` | ✅ | ✅ + `setSimpleFullscreen()` (macOS) | ✅ + `ToggleFullscreen()` |
| `setAlwaysOnTop()` | ✅ | ✅ | ✅ |
| `hide()` / `show()` | ✅ | ✅ | ✅ |
| `close()` | ✅ | ✅ + `destroy()` | ✅ |
| `setFocus()` | ✅ | ✅ | ✅ `Focus()` |
| `setEnabled()` | ✅ | ✅ | ✅ |

### 1.2 Window Appearance

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Frameless / borderless | ✅ `frameless: true` | ✅ | ✅ + `ToggleFrameless()` |
| Transparent | ✅ `transparent: true` | ✅ | ✅ |
| Custom titlebar (CSS drag) | ✅ `-webkit-app-region: drag` | ✅ `data-tauri-drag-region` | ✅ `--wails-draggable: drag` |
| App icon | ✅ `icon: "./icon.png"` | ✅ `setIcon()` | ✅ |
| Resizable lock | ✅ `resizable: false` | ✅ `setResizable()` | ✅ `DisableResize` / `SetResizable()` |
| Window decorations toggle | ✅ `setDecorations(bool)` | ✅ `setDecorations()` | ✅ via frameless |
| Window shadow | ✅ `setShadow(bool)` | ✅ `setShadow()` | ✅ macOS `DisableShadow` |
| Background color | ✅ `setBackgroundColor(r,g,b,a)` | ✅ `setBackgroundColor()` | ✅ `SetBackgroundColour()` |
| Title bar style (macOS) | ✅ `setTitleBarStyle("transparent"\|"overlay"\|"hidden")` | ✅ `setTitleBarStyle()` | ✅ `MacTitleBar` options |
| Hidden title (macOS) | ✅ via `setTitleBarStyle("overlay")` | ✅ `hiddenTitle` | ✅ |
| Window effects (Mica/Acrylic/Blur) | ✅ `vibrancy: "mica"` (Win11 Mica/Acrylic/Tabbed, macOS NSVisualEffect) | ✅ 28 variants via `setEffects()` | ✅ Mica/Acrylic/Tabbed + macOS LiquidGlass |
| Window mask (custom shape) | ❌ | ❌ | ✅ Windows `WindowMask` |

### 1.3 Advanced Window Behavior

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Multi-window | ✅ `app.createWindow()` | ✅ | ✅ |
| `setAlwaysOnBottom()` | ✅ | ✅ | ❌ |
| `setSkipTaskbar()` | ✅ | ✅ | ✅ Windows `HiddenOnTaskbar` |
| `setContentProtected()` (no screenshots) | ✅ | ✅ | ✅ `ContentProtectionEnabled` |
| `setProgressBar()` (taskbar) | ✅ Win/macOS badge | ✅ | ❌ |
| `setVisibleOnAllWorkspaces()` | ❌ | ✅ | ❌ |
| `setFocusable()` | ❌ | ✅ | ❌ |
| `setMovable()` | ❌ | ✅ | ❌ |
| `setClosable()` / `setMaximizable()` / `setMinimizable()` | ✅ `window.setButtons({minimize, maximize, close})` | ✅ | ✅ via button state |
| `setAutoResize()` (to content) | ❌ | ✅ | ❌ |
| Badge count (dock/taskbar) | ✅ `app.window.setBadgeCount(n)` (macOS) | ✅ `setBadgeCount()`, `setBadgeLabel()` | ❌ |
| Overlay icon (Windows) | ❌ | ✅ `setOverlayIcon()` | ❌ |
| `requestUserAttention()` | ✅ | ✅ | ✅ `Flash()` |
| Hide on focus lost | ❌ | ❌ | ✅ `HideOnFocusLost` |
| Hide on Escape | ❌ | ❌ | ✅ `HideOnEscape` |
| Snap assist (Windows) | ❌ | ❌ | ✅ `SnapAssist()` |
| Print page | ❌ | ❌ | ✅ `Print()` |
| Inject CSS | ❌ | ❌ | ✅ `CSS` option |
| Inject JS at creation | ❌ | ❌ | ✅ `JS` option |
| Attach modal window | ❌ | ❌ | ✅ `AttachModal()` |
| Reparent webview | ❌ | ✅ `reparent()` | ❌ |

### 1.4 Cursor Management

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| `setCursorGrab()` | ❌ | ✅ | ❌ |
| `setCursorVisible()` | ❌ | ✅ | ❌ |
| `setCursorIcon()` | ❌ | ✅ | ❌ |
| `setCursorPosition()` | ❌ | ✅ | ❌ |
| `setIgnoreCursorEvents()` | ❌ | ✅ | ✅ |

### 1.5 Dragging

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| `startDragging()` (programmatic) | ❌ | ✅ | ❌ |
| `startResizeDragging()` | ❌ | ✅ | ❌ |

### 1.6 Window Events

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| `onMoved` | ✅ | ✅ | ✅ `WindowDidMove` |
| `onResized` | ✅ | ✅ | ✅ `WindowDidResize` |
| `onFocusChanged` | ✅ | ✅ `onFocusChanged` | ✅ `WindowFocus` / `WindowLostFocus` |
| `onCloseRequested` | ✅ `onClose()` | ✅ | ✅ `WindowClosing` |
| `onScaleChanged` (DPI) | ❌ | ✅ | ✅ `WindowDPIChanged` |
| `onThemeChanged` | ✅ | ✅ | ✅ `ThemeChanged` |
| `onDragDrop` events | ✅ `fileDrop` + `fileDragEnter` + `fileDragLeave` | ✅ (enter/over/drop/leave) | ✅ `WindowFilesDropped` |

### 1.7 Window State Persistence

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Save/restore size | ✅ `windowState: true` | ✅ plugin | ❌ |
| Save/restore position | ✅ | ✅ plugin | ❌ |
| Save/restore maximized | ❌ | ✅ flags | ❌ |
| Save/restore fullscreen | ❌ | ✅ flags | ❌ |

---

## 2. Screen / Monitor API

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| List all monitors | ✅ `app.window.getMonitors()` | ✅ `availableMonitors()` | ✅ `GetAll()` |
| Get primary monitor | ✅ (primary flag in results) | ✅ `primaryMonitor()` | ✅ `GetPrimary()` |
| Get current monitor | ❌ | ✅ `currentMonitor()` | ✅ `GetCurrent()` (from window) |
| Monitor from point | ❌ | ✅ `monitorFromPoint()` | ❌ |
| Cursor position (global) | ❌ | ✅ `cursorPosition()` | ❌ |
| DPI/physical conversion | ❌ | ✅ `dpi` module | ✅ `DipToPhysicalPoint/Rect()` |
| Window positioner (preset positions) | ✅ `app.window.position("topRight")` (11 positions) | ✅ positioner plugin (15 positions) | ❌ |

---

## 3. Dialogs

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Open file | ✅ `app.dialog.open()` | ✅ | ✅ |
| Save file | ✅ `app.dialog.save()` | ✅ | ✅ |
| Open directory | ✅ `app.dialog.directory()` | ✅ | ✅ |
| File filters | ✅ | ✅ | ✅ |
| Multiple selection | ✅ | ✅ | ✅ |
| Alert (message) | ✅ `app.dialog.message()` | ✅ | ✅ `DialogInfo` |
| Confirm (OK/Cancel) | ✅ `app.dialog.confirm()` | ✅ `ask()`, `confirm()` | ✅ `DialogQuestion` |
| Input (prompt) | ✅ `app.dialog.input()` overlay HTML natif | ❌ | ❌ |
| Warning dialog | ❌ | ✅ kind: "warning" | ✅ `DialogWarning` |
| Error dialog | ❌ | ✅ kind: "error" | ✅ `DialogError` |
| Custom button labels | ❌ | ✅ `okLabel`, `cancelLabel` | ✅ `AddButton()` |
| Attach to window (modal) | ❌ | ❌ | ✅ `AttachToWindow()` |
| Show hidden files | ❌ | ❌ | ✅ `ShowHiddenFiles()` |
| Create directories from dialog | ❌ | ✅ `canCreateDirectories` | ✅ |
| Resolve aliases | ❌ | ❌ | ✅ macOS `ResolvesAliases()` |

---

## 4. System Tray

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Create tray icon | ✅ `app.tray.create()` | ✅ `TrayIcon.new()` | ✅ |
| Set tooltip | ✅ | ✅ `setTooltip()` | ✅ `SetTooltip()` |
| Set icon | ✅ | ✅ `setIcon()` | ✅ `SetIcon()` |
| Set menu | ✅ `app.tray.setMenu()` | ✅ `setMenu()` | ✅ `SetMenu()` |
| Remove tray | ✅ `app.tray.remove()` | ✅ `close()` | ✅ `Destroy()` |
| Click event | ✅ `app.tray.onClick()` | ✅ | ✅ `OnClick()` |
| Menu item click | ✅ `app.tray.onMenuClick()` | ✅ | ✅ |
| Set title (macOS) | ❌ | ✅ `setTitle()` | ✅ `SetLabel()` |
| Template icon (macOS) | ❌ | ✅ `setIconAsTemplate()` | ✅ `SetTemplateIcon()` |
| Dark mode icon | ❌ | ❌ | ✅ `SetDarkModeIcon()` |
| Right-click event | ❌ | ✅ | ✅ `OnRightClick()` |
| Double-click event | ❌ | ✅ | ✅ `OnDoubleClick()` |
| Mouse enter/leave | ❌ | ✅ | ✅ `OnMouseEnter()` / `OnMouseLeave()` |
| Show menu on left click | ❌ | ✅ `setShowMenuOnLeftClick()` | ❌ |
| Attach window to tray | ❌ | ❌ | ✅ `AttachWindow()` |
| Position window near tray | ❌ | ✅ positioner plugin | ✅ `PositionWindow()` |
| Show/hide tray | ❌ | ❌ | ✅ `Show()` / `Hide()` |

---

## 5. Menus

### 5.1 Application Menu Bar

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Set menu | ✅ `app.menu.set()` | ✅ `setAsAppMenu()` | ✅ `SetMenu()` |
| Remove menu | ✅ `app.menu.remove()` | ✅ | ✅ `HideMenuBar()` |
| Submenu (recursive) | ✅ | ✅ `Submenu` class | ✅ `AddSubmenu()` |
| Menu item click event | ✅ `app.menu.onClick()` | ✅ | ✅ `OnClick()` |
| Separator | ✅ | ✅ | ✅ `AddSeparator()` |
| Checkbox item | ✅ `checked` field | ✅ `CheckMenuItem` | ✅ `AddCheckbox()` |
| Radio item | ❌ | ❌ | ✅ `AddRadio()` |
| Enabled/disabled | ✅ | ✅ `setEnabled()` | ✅ `SetEnabled()` |
| Accelerator / shortcut key | ⚠️ label only | ✅ `setAccelerator()` | ✅ `SetAccelerator()` |
| Icon in menu item | ❌ | ✅ `IconMenuItem` (59 native icons) | ✅ `SetBitmap()` |
| Tooltip on item | ❌ | ❌ | ✅ `SetTooltip()` |
| Role-based items (Copy/Paste/Quit...) | ❌ | ✅ `PredefinedMenuItem` | ✅ 54 roles |
| Dynamic update (add/remove/insert) | ❌ (re-set entire menu) | ✅ `append()`, `prepend()`, `insert()`, `remove()` | ✅ `Append()`, `Prepend()` |
| Toggle menu bar visibility | ❌ | ❌ | ✅ `ToggleMenuBar()` |
| Clone menu | ❌ | ❌ | ✅ `Clone()` |

### 5.2 Context Menu

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Show context menu | ✅ `app.contextMenu.show()` | ✅ `menu.popup()` | ✅ `OpenContextMenu()` |
| Click event | ✅ `app.contextMenu.onClick()` | ✅ | ✅ |
| Context data passing | ❌ | ❌ | ✅ `data` parameter |
| Named context menus | ❌ | ❌ | ✅ `NewContextMenu(name)` |

---

## 6. Clipboard

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Read text | ✅ `app.clipboard.read()` | ✅ `readText()` | ✅ `Text()` |
| Write text | ✅ `app.clipboard.write()` | ✅ `writeText()` | ✅ `SetText()` |
| Read image | ❌ | ✅ `readImage()` | ❌ |
| Write image | ❌ | ✅ `writeImage()` | ❌ |
| Write HTML | ✅ `app.clipboard.writeHtml()` | ✅ `writeHtml()` | ❌ |
| Clear clipboard | ✅ `app.clipboard.clear()` | ✅ `clear()` | ❌ |
| Clipboard change monitoring | ❌ | ✅ event-based | ❌ |

---

## 7. Notifications

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Basic notification (title + body) | ✅ `app.notify()` | ✅ `sendNotification()` | ❌ |
| Custom icon | ⚠️ API exists | ✅ `icon`, `largeIcon` | ❌ |
| Permission check | ❌ | ✅ `isPermissionGranted()` | N/A |
| Action buttons | ❌ | ✅ `actionTypes` | ❌ |
| Interactive inputs | ❌ | ✅ | ❌ |
| Sound | ❌ | ✅ `sound` | ❌ |
| Schedule (delayed/recurring) | ❌ | ✅ `schedule` | ❌ |
| Notification groups | ❌ | ✅ `group` | ❌ |
| Cancel/remove notifications | ❌ | ✅ `cancel()`, `cancelAll()`, `removeActive()` | ❌ |
| Notification channels | ❌ | ✅ `createChannel()` | ❌ |
| On notification received | ❌ | ✅ `onNotificationReceived()` | ❌ |
| On action callback | ❌ | ✅ `onAction()` | ❌ |

---

## 8. Global Shortcuts / Hotkeys

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Register hotkey | ✅ `app.shortcut.register()` | ✅ `register()` | ✅ `Add()` |
| Unregister hotkey | ✅ `app.shortcut.unregister()` | ✅ `unregister()` | ✅ `Remove()` |
| Unregister all | ❌ | ✅ `unregisterAll()` | ❌ |
| Check if registered | ❌ | ✅ `isRegistered()` | ❌ |
| Triggered event | ✅ `app.shortcut.onTriggered()` | ✅ handler callback | ✅ callback |
| Key state (pressed/released) | ❌ | ✅ `ShortcutEvent.state` | ❌ |
| Wayland support | ❌ | ❌ | ❌ |

---

## 9. IPC & Communication

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Typed RPC | ✅ `export async function` + `client.rpc.*` proxy (zero codegen) | ✅ `#[tauri::command]` | ✅ Go struct bindings |
| Auto-generated TS types | ✅ native `typeof commands` (no build step) | ⚠️ via TauRPC | ✅ `wails generate bindings` |
| Events backend → frontend | ✅ `app.emit()` via `webview_eval` (zero TCP) | ✅ `emit()` | ✅ pub/sub |
| Events frontend → backend | ✅ `client.emit()` via native `__bv_emit__` binding | ✅ `listen()` | ✅ pub/sub |
| Bidirectional channels | ✅ `Channel<T>` (auto-wired event id, `.send()` / `.onMessage()`) | ✅ `Channel<T>` class | ❌ |
| Invoke timeout | ✅ configurable | ❌ | ❌ |
| Zero-network IPC (no HTTP/WS) | ✅ `webview_bind` + `webview_eval` — identical to Tauri | ✅ same approach | ❌ uses HTTP |
| `once()` listener | ✅ | ✅ | ❌ |
| eval() JS | ✅ `app.eval()` | ✅ | ✅ `ExecJS()` |
| Binary data (ArrayBuffer/Uint8Array) | ✅ transparent base64 auto-encode | ✅ native | ❌ |
| Emit to specific target | ❌ | ✅ `emitTo()` | ❌ |
| Call cancellation | ❌ | ❌ | ✅ `CancelCall()` |
| Event hooks (sync + cancel) | ❌ | ❌ | ✅ |
| Custom IPC transport | ❌ | ❌ | ✅ `Transport` option |

---

## 10. File System & Shell

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Shell open (URL/file) | ✅ `shell.open()` | ✅ `openUrl()`, `openPath()` | ✅ `OpenURL()` |
| Shell exec (command) | ✅ `shell.exec()` | ✅ `Command` class | ⚠️ via Go backend |
| Reveal in file manager | ❌ | ✅ `revealItemInDir()` | ✅ `OpenFileManager()` |
| File system API from frontend | ⚠️ via backend commands | ✅ full `fs` plugin (17 methods) | ⚠️ via Go backend |
| File watcher | ✅ `app.watch(path, cb, {recursive})` | ✅ `watch()`, `watchImmediate()` | ❌ |
| File handle (read/write/seek) | ❌ | ✅ `FileHandle` class | ❌ |
| Scoped file access | ❌ | ✅ with permissions | ❌ |
| HTTP client from frontend | ⚠️ via Bun `fetch` | ✅ plugin with proxy/cert options | ⚠️ via Go backend |
| WebSocket client | ⚠️ via native `WebSocket` / `ws` | ✅ plugin | ❌ |
| Download with progress | ✅ `downloadFile(url, {dest, onProgress})` atomic + AbortSignal | ✅ plugin | ❌ |
| Upload with progress | ⚠️ via `fetch` | ✅ plugin | ❌ |

---

## 11. Drag & Drop

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| File drop (native paths) | ✅ `app.onFileDrop()` | ✅ | ✅ `WindowFilesDropped` |
| Drag enter event | ❌ | ✅ `DRAG_ENTER` | ✅ `HandleDragEnter()` |
| Drag over event | ❌ | ✅ `DRAG_OVER` | ✅ `HandleDragOver()` |
| Drag leave event | ❌ | ✅ `DRAG_LEAVE` | ✅ `HandleDragLeave()` |
| HTML5 drag & drop | ✅ browser native | ✅ | ✅ |
| Enable/disable file drop | ❌ | ❌ | ✅ `EnableFileDrop` option |
| Drop zone targeting | ❌ | ❌ | ✅ zone identification |

---

## 12. Auto-Updater

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Check for updates | ✅ `autoUpdater.check()` | ✅ | ❌ |
| Download + install | ✅ `autoUpdater.downloadAndInstall()` | ✅ | ❌ |
| Progress callback | ✅ `onProgress` | ✅ | ❌ |
| Periodic auto-check | ✅ `autoUpdater.startAutoCheck()` | ✅ | ❌ |
| GitHub Releases support | ✅ | ✅ | ❌ |
| Signature verification | ❌ | ✅ | ❌ |
| Delta updates (diff) | ❌ | ✅ | ❌ |
| Custom update server | ❌ | ✅ (static JSON / dynamic) | ❌ |
| Allow downgrades | ❌ | ✅ `allowDowngrades` | ❌ |
| Proxy support | ❌ | ✅ `proxy` option | ❌ |

---

## 13. Autolaunch & Single Instance

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Enable autostart | ✅ `autolaunch.enable()` | ✅ plugin | ❌ |
| Disable autostart | ✅ `autolaunch.disable()` | ✅ plugin | ❌ |
| Check status | ✅ `autolaunch.isEnabled()` | ✅ plugin | ❌ |
| Single instance lock | ✅ `singleInstance: true` | ✅ plugin | ✅ |
| Focus existing on 2nd launch | ✅ `onSecondInstance` + auto-focus | ✅ callback | ✅ `OnSecondInstanceLaunch()` |
| Pass data to existing instance | ✅ `{argv, cwd, timestamp}` | ✅ (args, cwd) | ✅ `AdditionalData` |
| Encrypted communication | ❌ | ❌ | ✅ AES-256-GCM |

---

## 14. Deep Linking

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Custom URL protocol (`myapp://`) | ✅ `urlScheme` config | ✅ plugin | ❌ |
| Register protocol | ✅ auto at build time (NSIS / Info.plist / .desktop) | ✅ `register()` | ❌ |
| Check registration | ❌ | ✅ `isRegistered()` | ❌ |
| `onOpenUrl` handler | ✅ `app.onDeepLink(url)` | ✅ | ❌ |
| File associations | ❌ | ❌ | ✅ `FileAssociations` |

---

## 15. Persistent Storage

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Key-value store | ✅ `app.store.{get,set,delete,has,clear,keys,entries}()` (atomic JSON) | ✅ `Store` plugin (full CRUD + events) | ❌ |
| SQLite | ⚠️ via `bun:sqlite` (native) | ✅ `sql` plugin (SQLite/MySQL/PG) | ⚠️ via Go |
| Encrypted storage | ❌ | ✅ Stronghold plugin | ❌ |
| Persisted scope | ❌ | ✅ plugin | N/A |

---

## 16. Logging

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Structured logging | ✅ `app.log.{debug,info,warn,error}` | ✅ `log` plugin (5 levels) | ✅ via Go `slog` |
| Attach console | ✅ default `console: true` | ✅ `attachConsole()` | ❌ |
| Log levels | ✅ debug/info/warn/error | ✅ | ✅ |
| Log to file | ✅ with rotation (5 MB × 3) | ✅ | ✅ |
| Custom log level | ✅ `createLogger({level})` | ✅ | ✅ |

---

## 17. OS & Environment

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Platform name | ✅ `app.os.platform` | ✅ `platform()` | ✅ `Info()` |
| Architecture | ✅ `app.os.arch` | ✅ `arch()` | ✅ `Info()` |
| OS version | ✅ `app.os.version()` | ✅ `version()` | ✅ `Info()` |
| Hostname | ✅ `app.os.hostname()` | ✅ `hostname()` | ❌ |
| Locale (BCP-47) | ✅ `app.os.locale()` | ✅ `locale()` | ❌ |
| EOL character | ✅ `app.os.eol` | ✅ `eol()` | ❌ |
| OS family | ✅ `app.os.family` | ✅ `family()` | ❌ |
| Uptime / memory | ✅ `app.os.{uptime,totalMemory,freeMemory}()` | ⚠️ | ⚠️ |
| Dark mode detection | ✅ `app.onThemeChanged()` | ✅ `theme()` | ✅ `IsDarkMode()` |
| System accent color | ❌ | ❌ | ✅ `GetAccentColor()` |
| Process exit | ✅ `app.exit(code?)` | ✅ `exit()` | ✅ `Quit()` |
| Process relaunch | ✅ `app.relaunch()` | ✅ `relaunch()` | ❌ |
| CLI argument parsing | ✅ `app.cliArgs.{flags,positionals}` | ✅ `cli` plugin | ❌ |

---

## 18. Path Utilities

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| App data dir | ✅ `app.paths.data()` | ✅ `appDataDir()` | ❌ |
| App config dir | ✅ `app.paths.config()` | ✅ `appConfigDir()` | ❌ |
| App cache dir | ✅ `app.paths.cache()` | ✅ `appCacheDir()` | ❌ |
| App log dir | ✅ `app.paths.logs()` | ✅ `appLogDir()` | ❌ |
| Home dir | ✅ `app.paths.home()` | ✅ `homeDir()` | ❌ |
| Temp dir | ✅ `app.paths.temp()` | ✅ `tempDir()` | ❌ |
| Desktop dir | ✅ `app.paths.desktop()` | ✅ `desktopDir()` | ❌ |
| Downloads dir | ✅ `app.paths.downloads()` | ✅ `downloadDir()` | ❌ |
| Documents dir | ✅ `app.paths.documents()` | ✅ `documentDir()` | ❌ |
| Pictures / Music / Videos | ✅ | ✅ | ❌ |
| Current executable | ✅ `app.paths.executable()` | ✅ | ❌ |
| Auto-create missing dirs | ✅ on first access | ⚠️ manual | N/A |
| XDG-compliant on Linux | ✅ honors `$XDG_*_HOME` | ✅ | N/A |
| Path manipulation | ✅ `import path` | ✅ 10 functions | ❌ |

---

## 19. Security

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Permissions / ACL | ❌ | ✅ capability-based | ❌ |
| Command whitelisting | ❌ | ✅ | ❌ |
| Scoped file access | ❌ | ✅ allow/deny patterns | ❌ |
| Scoped HTTP access | ❌ | ✅ URL patterns | ❌ |
| CSP (Content Security Policy) | ✅ auto-injected in `bv://` handler | ✅ auto-injected | ❌ |
| Default-deny model | ❌ | ✅ | ❌ |
| `checkPermissions()` | ❌ | ✅ | ❌ |
| `requestPermissions()` | ❌ | ✅ | ❌ |

**Note:** bunview's security model is **structural, not declarative**. The exposure surface is the `commands` module passed to `createApp({ commands })` — anything you don't export from that module is unreachable from the frontend IPC. Keep internal helpers in a separate file (`backend/internal.ts`) and they're invisible to the webview by construction. No config, no ACL list to maintain, no desync between code and policy. Tauri-style fine-grained per-resource scopes (file path patterns, URL allowlists) are out of scope — if you need them, enforce at the command level.

---

## 20. Build & Distribution

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Single-file standalone binary | ✅ embedded assets | ❌ | ❌ |
| Portable .zip | ✅ auto-generated | ❌ | ❌ |
| Windows installer (NSIS) | ✅ `--installer` (auto-dl) | ✅ | ✅ |
| Windows MSI (WiX) | ✅ WiX v3 auto-downloaded (no install needed) | ✅ | ❌ |
| macOS .app bundle | ✅ | ✅ | ✅ |
| macOS .dmg | ✅ via hdiutil | ✅ | ✅ |
| Linux .deb | ✅ | ✅ | ❌ |
| Linux AppImage | ✅ via appimagetool | ✅ | ✅ |
| Linux .rpm | ✅ nfpm auto-downloaded (no `rpmbuild` needed) | ✅ | ❌ |
| Code signing (Windows) | ✅ signtool via `codeSigning.windows` | ✅ | ✅ |
| Code signing (macOS) | ✅ codesign + hardened runtime | ✅ | ✅ |
| macOS notarization | ✅ `notarytool submit` + `stapler staple` | ✅ | ❌ |
| Cross-compilation | ✅ Bun app | ✅ | ✅ |
| PE patch (no console) | ✅ auto | ✅ | ✅ |
| Icon embedding | ❌ | ✅ | ✅ |
| Auto icon generation (PNG → ICO/ICNS) | ✅ via `png2icons` (SVG or PNG source) | ✅ | ✅ CLI tool |
| WebView2 runtime installer (Windows) | ❌ | ✅ bundled | ❌ |
| Build hooks (before/after) | ❌ | ✅ | ✅ |

---

## 21. Developer Experience

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| CLI init / scaffold | ✅ `bunview init` | ✅ `tauri init` | ✅ `wails init` |
| Auto-detect frontend framework | ✅ (8 build tools, 8 UI libs) | ❌ | ❌ |
| Init on existing project | ✅ (zero config modification) | ❌ | ❌ |
| Reject server-side frameworks | ✅ (16 frameworks detected) | ❌ | ❌ |
| Dev mode with HMR | ✅ auto-proxy to dev server | ✅ | ✅ |
| Hot reload backend | ✅ `bun --watch` | ❌ manual restart | ✅ |
| 100% TypeScript backend | ✅ | ❌ (Rust) | ❌ (Go) |
| Zero-config for any framework | ✅ | ❌ | ❌ |
| Template system | ⚠️ vanilla scaffold only | ✅ | ✅ |
| Plugin development framework | ❌ | ✅ | ✅ |

---

## 22. Hardware Introspection

Native hardware queries from the backend — CPU, GPU, RAM, battery, disks, network, USB, audio, displays, processes, users, temperatures.

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| CPU info (model, cores, arch) | ✅ `getSystemInfo()` + `getCpuDetails()` (L2/L3 cache, max MHz, vendor) | ⚠️ via `os-info` plugin | ⚠️ Go `runtime` |
| CPU usage (per-core %) | ✅ `getCpuUsage()` two-sample | ❌ | ❌ |
| Memory info | ✅ `getMemoryInfo()` (used/free/swap) | ❌ | ❌ |
| RAM modules (DDR type, speed, manufacturer) | ✅ `getRamDetails()` — WMI / system_profiler / dmidecode | ❌ | ❌ |
| Battery | ✅ `getBatteryInfo()` via `starship-battery` (cross-platform) | ⚠️ frontend only | ❌ |
| Disks (space, SSD/HDD, FS) | ✅ `getDiskInfo()` via `sysinfo` | ❌ | ❌ |
| Network interfaces (MAC + IPs) | ✅ `getNetworkInfo()` via `sysinfo` + `if-addrs` | ❌ | ❌ |
| Network speed (rx/tx bytes/s) | ✅ `getNetworkSpeed()` two-sample | ❌ | ❌ |
| USB devices (VID/PID/class) | ✅ `getUsbDevices()` via `nusb` (pure Rust) | ❌ | ❌ |
| Audio devices (input/output) | ✅ `getAudioDevices()` WMI/system_profiler/pactl | ❌ | ❌ |
| Displays (resolution, refresh) | ✅ `getDisplayInfo()` | ⚠️ `primaryMonitor()` | ⚠️ `ScreenGetAll()` |
| Processes (top 50 by CPU) | ✅ `getProcessList()` via `sysinfo` | ❌ | ❌ |
| System users | ✅ `getUsers()` via `sysinfo` | ❌ | ❌ |
| Temperatures (CPU + all sensors) | ✅ `getTemperature()` sysinfo + WMI fallback | ❌ | ❌ |
| Real-time monitoring (1Hz…) | ✅ `startMonitoring()` → `hwMonitorUpdate` events | ❌ | ❌ |

---

## 23. GPU & AI Acceleration

### GPU introspection

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| GPU list (name, VRAM, type) | ✅ `getSystemInfo().gpus` | ❌ | ❌ |
| GPU utilization % | ✅ `getGpuUsage()` (nvidia-smi / WMI / rocm-smi) | ❌ | ❌ |
| VRAM used / total | ✅ | ❌ | ❌ |
| GPU temperature / power | ✅ | ❌ | ❌ |

### AI capabilities (`getAiCapabilities()`)

| Capability | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| CUDA + compute capability + version | ✅ via nvidia-smi | ❌ | ❌ |
| ROCm + version | ✅ via rocm-smi + rocminfo | ❌ | ❌ |
| Vulkan | ✅ via vulkaninfo | ❌ | ❌ |
| Metal (macOS) | ✅ | ❌ | ❌ |
| DirectML (Windows) | ✅ DirectML.dll detection | ❌ | ❌ |
| WebNN + active backends | ✅ `{backends: ["directml"]}` / `["coreml", "ane"]` | ❌ | ❌ |
| Intel NPU (AI Boost / VPU) | ✅ WMI PnPEntity / `/sys/class/accel` vendor 0x8086 | ❌ | ❌ |
| Apple Neural Engine | ✅ `sysctl hw.optional.ane_version` | ❌ | ❌ |
| AMD XDNA (Ryzen AI) | ✅ WMI / `amdxdna` driver | ❌ | ❌ |
| Qualcomm Hexagon NPU | ✅ WMI (Snapdragon X) | ❌ | ❌ |

### Frontend GPU/NPU acceleration

| Feature | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| WebGPU in WebView | ✅ `hardwareAcceleration: true` → D3D12 / Metal / Vulkan | ⚠️ OS default | ⚠️ OS default |
| WebNN in WebView | ✅ `--enable-features=WebMachineLearningNeuralNetwork` — routes to NPU via DirectML / CoreML | ❌ | ❌ |
| SharedArrayBuffer + WASM threads | ✅ COOP/COEP headers injected in `bv://` handler | ⚠️ manual | ❌ |
| Transformers.js / ONNX Runtime WebGPU backend | ✅ out-of-the-box when `hardwareAcceleration: true` | ⚠️ requires manual CSP | ❌ |
| Opt-in toggle per app | ✅ `WindowConfig.hardwareAcceleration` (default `false`) | ❌ (always on) | ❌ |

**bunview is currently the only desktop framework with first-class hardware introspection and NPU-aware WebView configuration.**

---

## 24. IPC Architecture (zero-network, production only)

This section documents how each framework delivers IPC without a TCP round-trip.

| Mechanism | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| JS → Native binding | ✅ wry `window.ipc.postMessage` + JS shim (`window.__bunview__`) | ✅ `invoke_handler` (same wry mechanism) | ❌ HTTP POST to localhost |
| Native → JS push | ✅ `webview.evaluate_script(dispatch call)` | ✅ `webview.evaluate_script` | ❌ WebSocket to localhost |
| Frontend → Backend events | ✅ `window.__bv_emit__` via `window.ipc.postMessage` (native) | ✅ native | ❌ WebSocket |
| No TCP / No Firewall popup | ✅ | ✅ | ❌ |
| Windows file serving | ✅ wry `with_custom_protocol("bv", …)` → WebView2 custom scheme | ✅ same wry mechanism | ❌ HTTP |
| macOS file serving | ✅ wry `with_custom_protocol("bv", …)` → WKURLSchemeHandler | ✅ same | ❌ HTTP |
| Linux file serving | ✅ wry `with_custom_protocol("bv", …)` → webkit_web_context_register_uri_scheme | ✅ same | ❌ HTTP |
| SPA routing (deep links) | ✅ all 3 platforms via scheme handler fallback | ✅ | ✅ (HTTP 200 always) |
| `window.location.origin` | ✅ `bv://localhost` | ✅ `tauri://localhost` | ✅ `http://localhost:port` |
| Native host language | ✅ Rust (wry + tao) | ✅ Rust (wry + tao) | ❌ Go |

**Conclusion:** bunview and Tauri v2 use the **same underlying stack** (wry + tao + custom URI scheme) for zero-network, zero-TCP communication. The bunview host binary was rewritten from C++ to Rust in April 2026, achieving full parity with Tauri's IPC architecture. Wails v3 still uses localhost HTTP/WebSocket.

---

## 25. Unique Features (not in other frameworks)

### bunview exclusives

| Feature | Description |
|---|---|
| **Single-file standalone .exe** | Everything embedded (frontend + webview-host), no installer needed |
| **Zero-codegen typed RPC** | `export async function` in backend → `client.rpc.*` typed proxy in frontend. No build step. |
| **Hardware introspection** | CPU/GPU/RAM/battery/disks/network/USB/audio/displays/processes/temperatures — 17 APIs |
| **NPU detection** | Intel AI Boost, AMD XDNA, Apple Neural Engine, Qualcomm Hexagon |
| **WebNN in WebView** | Routes ML inference to NPU via DirectML (Windows) or CoreML/ANE (macOS) |
| **WebGPU toggle** | Opt-in `hardwareAcceleration: true` — safe default, power-saving |
| **Real-time HW monitoring** | `startMonitoring()` emits CPU/memory/temperatures/GPU/network at 1Hz |
| **Built-in path + store + logger + watcher** | `app.paths.*`, `app.store.*`, `app.log.*`, `app.watch()` — no plugins to install |
| **Built-in CLI args parser** | `app.cliArgs.flags` / `.positionals` — zero deps |
| **Built-in HTTP download with progress** | `downloadFile()` atomic + AbortSignal |
| **Tree-shakable subpath exports** | `bunview/client`, `bunview/shell`, `bunview/http`, `bunview/binary` — ship only what you use |
| **Transparent binary IPC** | `Uint8Array` in RPC auto-encoded, no manual base64 |
| **Auto-detect 8+ build tools** | Vite, CRA, Angular, Parcel, Rsbuild, Webpack detected automatically |
| **Init on existing project** | `bunview init` in a Vite/React project — adds bunview without modifying any config |
| **Reject server frameworks** | Detects Next.js, Nuxt, Remix, SvelteKit, etc. and blocks with clear message |
| **100% TypeScript backend** | No Rust/Go to write — full Bun runtime with `bun:sqlite`, native fetch, etc. |
| **Input dialog** | API disponible (`app.dialog.input()`) — implémentation HTML overlay cross-platform |
| **Invoke timeout** | Configurable timeout on RPC calls — not in Tauri or Wails |
| **Portable .zip** | Auto-generated compressed distributable |
| **Tools auto-downloaded** | NSIS, appimagetool downloaded on demand, nothing to install |
| **Multi-window via process isolation** | Each window is a separate native process — crash-proof |
| **Window positioner** | 11 preset positions (topLeft, trayRight, etc.) on any monitor |

### Tauri v2 exclusives

| Feature | Description |
|---|---|
| **Capability-based ACL** | Fine-grained permissions per command/resource |
| **30+ official plugins** | Huge ecosystem (stronghold, biometric, NFC, etc.) |
| **28 window effects** | Mica, Acrylic, Blur, Vibrancy, etc. |
| **59 native menu icons** | macOS NSImage system icons |
| **Cursor management** | Grab, icon, position, visibility |
| **Badge/overlay icons** | Taskbar badges and overlays |
| **Delta updates** | Binary diff for minimal download size |
| **Path utilities (23 dirs)** | All standard OS directories |
| **Persisted scope** | Auto-save/restore file access permissions |

### Wails v3 exclusives

| Feature | Description |
|---|---|
| **Server mode** | Serve app to regular browser clients via HTTP |
| **LiquidGlass** | macOS Sequoia glass effect |
| **SnapAssist** | Windows 11 snap layout trigger |
| **Print()** | Native page printing |
| **Window mask** | Custom window shape (Windows) |
| **54 menu roles** | All standard macOS/Windows menu actions |
| **File associations** | Register as handler for file extensions |
| **Event hooks** | Synchronous hooks with cancellation |
| **Custom IPC transport** | Replace default IPC mechanism |
| **Encrypted single instance** | AES-256-GCM between instances |
| **Inject CSS/JS at creation** | Per-window CSS/JS injection |
| **HideOnFocusLost / HideOnEscape** | Auto-hide behavior |

---

## Summary Score

| Category | bunview | Tauri v2 | Wails v3 |
|---|---|---|---|
| Window management (basic) | 14/14 | 14/14 | 14/14 |
| Window management (advanced) | 13/20 | 18/20 | 14/20 |
| Window events | 6/7 | 7/7 | 7/7 |
| Dialogs | 8/8 + input | 7/8 | 8/8 |
| System tray | 7/14 | 11/14 | 14/14 |
| Menus | 8/15 | 13/15 | 15/15 |
| Clipboard | 4/7 | 7/7 | 2/7 |
| Notifications | 2/12 | 12/12 | 0/12 |
| Global shortcuts | 4/6 | 6/6 | 4/6 |
| IPC | 12/14 | 13/14 | 9/14 |
| File system / Shell | 6/10 | 10/10 | 4/10 |
| Drag & drop | 5/7 | 6/7 | 6/7 |
| Auto-updater | 5/10 | 10/10 | 0/10 |
| Build & distribution | 14/18 | 15/18 | 10/18 |
| DX / CLI | 8/10 | 5/10 | 6/10 |
| OS & Environment | 11/12 | 12/12 | 6/12 |
| Path utilities | 12/13 | 13/13 | 1/13 |
| Persistent storage | 2/4 | 4/4 | 0/4 |
| Logging | 5/5 | 5/5 | 2/5 |
| Deep linking | 3/5 | 4/5 | 0/5 |
| Single instance | 4/6 | 6/6 | 4/6 |
| Security | 1/8 | 8/8 | 0/8 |
| **Hardware introspection** | **15/15** | **0/15** | **0/15** |
| **GPU & AI acceleration** | **15/15** | **1/15** | **1/15** |
| **Total** | **~184/245 (75%)** | **~210/245 (86%)** | **~128/245 (52%)** |

**IPC architecture parity with Tauri v2** — both use wry `window.ipc.postMessage` + `evaluate_script` + custom URI scheme (`bv://`) for zero-TCP communication.

**bunview unique edges**:
1. Only framework of the three with first-class **hardware introspection** (17 APIs) and **NPU-aware WebView** configuration (WebNN + DirectML/CoreML routing)
2. **Zero-codegen typed RPC** — `client.rpc.*` via `typeof commands` (no `tauri generate`, no Go bindings)
3. **Built-in path + store + logger + watcher + HTTP download** — no plugin install needed
4. **Tree-shakable subpath exports** — ship only what you use

The gap with Tauri is primarily in **ecosystem** (30+ plugins) and **Tauri-specific niceties** (28 window effects, cursor API, 23-path utility plugin). Security is not a gap — bunview uses a structural model (module boundary = exposure boundary), which is simpler and impossible to desynchronize from the code.
