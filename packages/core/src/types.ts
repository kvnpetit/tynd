import * as v from "valibot";

/**
 * Multi-GPU adapter pin. Workstations with hybrid graphics (NVIDIA Optimus,
 * AMD switchable, Intel + dGPU) can ask the host to wake the right device
 * before WebView2 / WKWebView / WebKitGTK initialise.
 *
 * - `"auto" | "primary"` — defer to the OS power-management heuristic.
 * - `"discrete"` — wake the discrete GPU (Linux: `__NV_PRIME_RENDER_OFFLOAD=1`
 *   + `DRI_PRIME=1`; Windows / macOS: handled via Chromium's
 *   `--force_high_performance_gpu` flag).
 * - `{ vendor: "nvidia"|"amd"|"intel" }` — vendor-specific env hints
 *   (Linux only; Win/macOS ignore the vendor and fall back to OS defaults).
 */
const GpuPreferenceSchema = v.union([
  v.picklist(["auto", "primary", "discrete"]),
  v.object({ vendor: v.picklist(["nvidia", "amd", "intel"]) }),
]);
export type GpuPreference = v.InferOutput<typeof GpuPreferenceSchema>;

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
  userAgent: v.optional(v.pipe(v.string(), v.minLength(1))),

  // ─────────── GPU acceleration knobs (all optional, sane defaults) ───────────
  /** Native vibrancy / blur material — see host-rs docs for per-OS coverage. */
  vibrancy: v.optional(v.string()),
  /**
   * Enable WebGPU. Defaults to `true`. Disable per-app only when the
   * workload is incompatible with waking the GPU process at all
   * (e.g. battery-sensitive utility apps). Three.js, Babylon, and any
   * WebGL-only stack keep working when `webgpu: false`.
   *
   * On macOS, this flag is ignored — WebKit does not expose a runtime
   * toggle. WebGPU availability is determined by macOS version (15+)
   * and the user's Safari Technology Preview settings.
   */
  webgpu: v.optional(v.boolean()),
  /**
   * Hardware-acceleration tier hint. Wakes the matching GPU on hybrid
   * laptops:
   * - `"high-performance"` → Chromium `--force_high_performance_gpu` +
   *   ANGLE D3D11 + GPU rasterization (Win) / discrete GPU pin (Linux).
   * - `"low-power"` → Chromium `--force_low_power_gpu`.
   * - `"default"` → defer to the OS heuristic.
   */
  hardwareAcceleration: v.optional(
    v.picklist(["default", "high-performance", "low-power"]),
  ),
  /**
   * Refresh-rate hint. `"monitor"` (default) adopts the monitor's native
   * rate; an integer caps the Chromium frame-budget timer (clamped to
   * 15..=480 Hz). Above the monitor's native rate the OS compositor
   * silently caps and the host emits a warn log.
   */
  preferredFps: v.optional(
    v.union([v.literal("monitor"), v.pipe(v.number(), v.integer(), v.minValue(1))]),
  ),
  /**
   * Auto-reload the WebView when the GPU / web-content process dies
   * (macOS / iOS only — Win/Linux compositors recover transparently).
   * Defaults to `true`. Apps that own the recovery flow can opt out and
   * listen to `webview:context-lost` directly.
   */
  contextLostRecovery: v.optional(v.boolean()),
  /**
   * Wide-gamut color-space tag for the swap chain / NSWindow / GTK widget.
   * Combine with `hdr: true` for HDR10 surfaces on Windows.
   */
  colorSpace: v.optional(v.picklist(["srgb", "p3", "rec2020", "linear"])),
  /** Request an HDR10 surface (Win / macOS). Linux logs a degraded fallback. */
  hdr: v.optional(v.boolean()),
  /**
   * Variable-refresh-rate (G-Sync / FreeSync / ProMotion). Off by default
   * — only enable for animation / gaming workloads that benefit from
   * tear-free dynamic frame pacing.
   */
  vrr: v.optional(v.boolean()),
  /** Multi-GPU adapter pin for workstations. See {@link GpuPreference}. */
  gpu: v.optional(GpuPreferenceSchema),
});
export type WindowConfig = v.InferOutput<typeof WindowConfigSchema>;

/** Native OS roles for menu items. Role items are managed by the OS. */
const MENU_ROLES = [
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
] as const;
export type MenuRole = (typeof MENU_ROLES)[number];

const MenuSeparatorSchema = v.object({ type: v.literal("separator") });
export type MenuSeparator = v.InferOutput<typeof MenuSeparatorSchema>;

const MenuActionSchema = v.object({
  type: v.optional(v.literal("item")),
  id: v.optional(v.string()),
  label: v.string(),
  enabled: v.optional(v.boolean()),
  role: v.optional(v.picklist(MENU_ROLES)),
  /** Keyboard accelerator (muda format — e.g. `"CmdOrCtrl+S"`, `"Alt+F4"`). */
  accelerator: v.optional(v.string()),
});
export type MenuAction = v.InferOutput<typeof MenuActionSchema>;

const MenuCheckboxSchema = v.object({
  type: v.literal("checkbox"),
  id: v.optional(v.string()),
  label: v.string(),
  enabled: v.optional(v.boolean()),
  checked: v.optional(v.boolean()),
  accelerator: v.optional(v.string()),
});
export type MenuCheckbox = v.InferOutput<typeof MenuCheckboxSchema>;

/** Radio items behave like checkboxes; enforce single-selection in userland by
 * updating `checked` on click (native OS doesn't group muda check items). */
const MenuRadioSchema = v.object({
  type: v.literal("radio"),
  id: v.optional(v.string()),
  label: v.string(),
  enabled: v.optional(v.boolean()),
  checked: v.optional(v.boolean()),
  accelerator: v.optional(v.string()),
});
export type MenuRadio = v.InferOutput<typeof MenuRadioSchema>;

// Recursive — declare the type first, then the schema references itself.
export interface MenuSubmenu {
  type: "submenu";
  label: string;
  enabled?: boolean | undefined;
  items: MenuItem[];
}
export type MenuItem = MenuSeparator | MenuAction | MenuCheckbox | MenuRadio | MenuSubmenu;

const MenuItemSchema: v.GenericSchema<MenuItem> = v.lazy(() =>
  v.union([
    MenuSeparatorSchema,
    MenuActionSchema,
    MenuCheckboxSchema,
    MenuRadioSchema,
    v.object({
      type: v.literal("submenu"),
      label: v.string(),
      enabled: v.optional(v.boolean()),
      items: v.array(MenuItemSchema),
    }),
  ]),
);

const TrayConfigSchema = v.object({
  icon: v.pipe(v.string(), v.minLength(1)),
  tooltip: v.optional(v.string()),
  menu: v.optional(v.array(MenuItemSchema)),
});
export type TrayConfig = v.InferOutput<typeof TrayConfigSchema>;

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
  /**
   * Exit the process as soon as the last window closes. Leave off (default)
   * for tray-only apps that should stay alive after all windows are hidden.
   */
  quitOnLastWindowClosed: v.optional(v.boolean()),
});
export type AppConfig = v.InferOutput<typeof AppConfigSchema>;

export interface NotificationOptions {
  /** Notification body text */
  body?: string;
  /** Absolute path to an icon file (PNG / ICO). */
  icon?: string;
  /** System sound name (`default`, `alarm`, ...). Platform-dependent. */
  sound?: string;
  /**
   * Action buttons. Click emits `notification:action` with the clicked
   * `id`. Works on all OS — Linux via libnotify, Windows via WinRT
   * toasts, macOS via NSUserNotification (single button = direct action,
   * 2+ = native dropdown).
   */
  actions?: { id: string; label: string }[];
}

export interface NotificationActionEvent {
  /** The `id` of the clicked action. */
  action: string;
}

export type EmitterMap = Record<string, unknown>;

/** Typed event emitter. Create via `createEmitter<T>()` in your backend. */
export interface Emitter<T extends EmitterMap> {
  /** @internal — type marker for BackendClient inference */
  readonly __tynd_emitter__: true;
  /** @internal — phantom type field, never has a real runtime value */
  readonly __tynd_event_types__: T;
  emit<K extends keyof T>(event: K & string, payload: T[K]): void;
  /**
   * Emit only to the window with the given `label`. Other windows don't
   * receive the event. Default label is `"main"` for the primary window.
   */
  emitTo<K extends keyof T>(label: string, event: K & string, payload: T[K]): void;
}

export interface FileFilter {
  /** Display name, e.g. "Images" */
  name: string;
  /** File extensions without the dot, e.g. ["png", "jpg"] */
  extensions: string[];
}

export interface OpenFileOptions {
  title?: string;
  defaultDir?: string;
  filters?: FileFilter[];
  /** macOS: allow the user to create directories from the picker. */
  canCreateDirectories?: boolean;
}

export interface SaveFileOptions {
  title?: string;
  /** Suggested file name */
  defaultName?: string;
  defaultDir?: string;
  filters?: FileFilter[];
  /** macOS: allow the user to create directories from the picker. */
  canCreateDirectories?: boolean;
}

export interface MessageOptions {
  title?: string;
  kind?: "info" | "warning" | "error";
}

export interface ConfirmOptions {
  title?: string;
}
