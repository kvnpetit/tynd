export interface WindowConfig {
  title?: string
  width?: number
  height?: number
  minWidth?: number
  minHeight?: number
  maxWidth?: number
  maxHeight?: number
  resizable?: boolean
  decorations?: boolean
  transparent?: boolean
  alwaysOnTop?: boolean
  center?: boolean
  fullscreen?: boolean
  maximized?: boolean
}

// ── Menu types ────────────────────────────────────────────────────────────────

/** Native OS roles for menu items. Role items are managed by the OS. */
export type MenuRole =
  | "quit"
  | "copy"
  | "cut"
  | "paste"
  | "undo"
  | "redo"
  | "selectAll"
  | "minimize"
  | "close"
  | "separator"
  | "about"

export interface MenuSeparator {
  type: "separator"
}

export interface MenuAction {
  type?: "item"
  /** Unique ID — emitted in `menu:action` event when clicked */
  id?: string
  label: string
  enabled?: boolean
  /** Native OS role. Overrides `id`/`label` if set. */
  role?: MenuRole
}

export interface MenuSubmenu {
  type: "submenu"
  label: string
  enabled?: boolean
  items: MenuItem[]
}

export type MenuItem = MenuSeparator | MenuAction | MenuSubmenu

// ── Tray types ────────────────────────────────────────────────────────────────

export interface TrayConfig {
  /** Path to tray icon (PNG/ICO/BMP) */
  icon: string
  tooltip?: string
  /** Context menu shown on right-click */
  menu?: MenuItem[]
}

// ── App config ────────────────────────────────────────────────────────────────

export interface AppConfig {
  window?: WindowConfig
  /** Native menu bar (macOS/Windows). Each top-level item must be a submenu. */
  menu?: MenuSubmenu[]
  /** System tray configuration */
  tray?: TrayConfig
  /** Dev server URL override. Auto-injected by CLI via VORN_DEV_URL. */
  devUrl?: string
  /** Frontend directory override. Auto-injected by CLI via VORN_FRONTEND_DIR. */
  frontendDir?: string
}

// ── Notification types ────────────────────────────────────────────────────────

export interface NotificationOptions {
  /** Notification body text */
  body?: string
}

export type EmitterMap = Record<string, unknown>

/** Typed event emitter. Create via `createEmitter<T>()` in your backend. */
export interface Emitter<T extends EmitterMap> {
  /** @internal — type marker for BackendClient inference */
  readonly __vorn_emitter__: true
  /** @internal — phantom type field, never has a real runtime value */
  readonly __vorn_event_types__: T
  emit<K extends keyof T>(event: K & string, payload: T[K]): void
}

// ── OS API types ──────────────────────────────────────────────────────────────

export interface FileFilter {
  /** Display name, e.g. "Images" */
  name: string
  /** File extensions without the dot, e.g. ["png", "jpg"] */
  extensions: string[]
}

export interface OpenFileOptions {
  title?: string
  defaultDir?: string
  filters?: FileFilter[]
}

export interface SaveFileOptions {
  title?: string
  /** Suggested file name */
  defaultName?: string
  defaultDir?: string
  filters?: FileFilter[]
}

export interface MessageOptions {
  title?: string
  kind?: "info" | "warning" | "error"
}

export interface ConfirmOptions {
  title?: string
}
