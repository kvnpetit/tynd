import * as v from "valibot"

const WindowConfigSchema = v.object({
  title: v.optional(v.string()),
  width: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  height: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  minWidth: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  minHeight: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  maxWidth: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  maxHeight: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  resizable: v.optional(v.boolean()),
  decorations: v.optional(v.boolean()),
  transparent: v.optional(v.boolean()),
  alwaysOnTop: v.optional(v.boolean()),
  center: v.optional(v.boolean()),
  fullscreen: v.optional(v.boolean()),
  maximized: v.optional(v.boolean()),
})
export type WindowConfig = v.InferOutput<typeof WindowConfigSchema>

/** Native OS roles for menu items. Role items are managed by the OS. */
export const MENU_ROLES = [
  "quit",
  "copy",
  "cut",
  "paste",
  "undo",
  "redo",
  "selectAll",
  "minimize",
  "close",
  "separator",
  "about",
] as const
export type MenuRole = (typeof MENU_ROLES)[number]

const MenuSeparatorSchema = v.object({ type: v.literal("separator") })
export type MenuSeparator = v.InferOutput<typeof MenuSeparatorSchema>

const MenuActionSchema = v.object({
  type: v.optional(v.literal("item")),
  id: v.optional(v.string()),
  label: v.string(),
  enabled: v.optional(v.boolean()),
  role: v.optional(v.picklist(MENU_ROLES)),
})
export type MenuAction = v.InferOutput<typeof MenuActionSchema>

// Recursive — declare the type first, then the schema references itself.
export interface MenuSubmenu {
  type: "submenu"
  label: string
  enabled?: boolean | undefined
  items: MenuItem[]
}
export type MenuItem = MenuSeparator | MenuAction | MenuSubmenu

const MenuItemSchema: v.GenericSchema<MenuItem> = v.lazy(() =>
  v.union([
    MenuSeparatorSchema,
    MenuActionSchema,
    v.object({
      type: v.literal("submenu"),
      label: v.string(),
      enabled: v.optional(v.boolean()),
      items: v.array(MenuItemSchema),
    }),
  ]),
)

const TrayConfigSchema = v.object({
  icon: v.pipe(v.string(), v.minLength(1)),
  tooltip: v.optional(v.string()),
  menu: v.optional(v.array(MenuItemSchema)),
})
export type TrayConfig = v.InferOutput<typeof TrayConfigSchema>

export const AppConfigSchema = v.object({
  window: v.optional(WindowConfigSchema),
  menu: v.optional(
    v.array(
      v.object({
        type: v.literal("submenu"),
        label: v.string(),
        enabled: v.optional(v.boolean()),
        items: v.array(MenuItemSchema),
      }),
    ),
  ),
  tray: v.optional(TrayConfigSchema),
  devUrl: v.optional(v.pipe(v.string(), v.url())),
  frontendDir: v.optional(v.pipe(v.string(), v.minLength(1))),
})
export type AppConfig = v.InferOutput<typeof AppConfigSchema>

export interface NotificationOptions {
  /** Notification body text */
  body?: string
}

export type EmitterMap = Record<string, unknown>

/** Typed event emitter. Create via `createEmitter<T>()` in your backend. */
export interface Emitter<T extends EmitterMap> {
  /** @internal — type marker for BackendClient inference */
  readonly __tynd_emitter__: true
  /** @internal — phantom type field, never has a real runtime value */
  readonly __tynd_event_types__: T
  emit<K extends keyof T>(event: K & string, payload: T[K]): void
}

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
