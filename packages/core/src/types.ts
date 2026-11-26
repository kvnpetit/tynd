import * as v from "valibot";

/**
 * Every `app.start()` option object is strict: an unknown key (a typo such as
 * `window: { titel }`) is rejected instead of being silently ignored. Open
 * records (`scope`) stay `v.unknown()`.
 */
function strictOptions<const TEntries extends v.ObjectEntries>(entries: TEntries) {
  const keys = Object.keys(entries).join(", ");
  return v.strictObject(
    entries,
    (issue) => `unknown option "${String(issue.input)}"; valid options: ${keys}`,
  );
}

/**
 * Multi-GPU adapter pin. Workstations with hybrid graphics (NVIDIA Optimus,
 * AMD switchable, Intel + dGPU) can ask the host to wake the right device
 * before WebView2 / WKWebView / WebKitGTK initialise.
 * - `"auto" | "primary"` — defer to the OS power-management heuristic.
 * - `"discrete"` — wake the discrete GPU (Linux: `__NV_PRIME_RENDER_OFFLOAD=1`
 *   + `DRI_PRIME=1`; Windows / macOS: handled via Chromium's
 *   `--force_high_performance_gpu` flag).
 * - `{ vendor: "nvidia"|"amd"|"intel" }` — vendor-specific env hints
 *   (Linux only; Win/macOS ignore the vendor and fall back to OS defaults).
 */
const GpuPreferenceSchema = v.union([
  v.picklist(["auto", "primary", "discrete"]),
  strictOptions({ vendor: v.picklist(["nvidia", "amd", "intel"]) }),
]);
export type GpuPreference = v.InferOutput<typeof GpuPreferenceSchema>;

/**
 * What the webview does with `window.open()` / `target="_blank"` requests.
 * - `"allow"` (default) — the platform opens an unmanaged popup (no IPC bridge),
 *   only for URLs that pass the navigation policy.
 * - `"external"` — http(s) and `mailto:` open in the system browser
 *   (throttled); every other scheme is dropped.
 * - `"deny"` — every request is dropped.
 * Each request emits `webview:new-window { label, url, action }`.
 */
export const NEW_WINDOW_POLICIES = ["deny", "external", "allow"] as const;
export type NewWindowPolicy = (typeof NEW_WINDOW_POLICIES)[number];

/** Download handling for a window or webview. */
export const DOWNLOAD_POLICIES = ["allow", "deny"] as const;
/** Decision for one web permission kind. */
export const WEB_PERMISSION_DECISIONS = ["allow", "deny", "prompt"] as const;
const WebPermissionDecisionSchema = v.optional(v.picklist(WEB_PERMISSION_DECISIONS));
const WebPermissionsSchema = strictOptions({
  camera: WebPermissionDecisionSchema,
  microphone: WebPermissionDecisionSchema,
  geolocation: WebPermissionDecisionSchema,
  notifications: WebPermissionDecisionSchema,
  clipboardRead: WebPermissionDecisionSchema,
  displayCapture: WebPermissionDecisionSchema,
  midi: WebPermissionDecisionSchema,
  sensors: WebPermissionDecisionSchema,
  localFonts: WebPermissionDecisionSchema,
  windowManagement: WebPermissionDecisionSchema,
  pointerLock: WebPermissionDecisionSchema,
  automaticDownloads: WebPermissionDecisionSchema,
  fileSystemAccess: WebPermissionDecisionSchema,
  autoplay: WebPermissionDecisionSchema,
});
/** `permissions` option: decision per web permission kind. */
export type WebPermissions = v.InferOutput<typeof WebPermissionsSchema>;
export type DownloadPolicy = (typeof DOWNLOAD_POLICIES)[number];

/**
 * Proxy endpoint: `http://host[:port]` (HTTP CONNECT; port defaults to 80) or
 * `socks5://host:port` (explicit port required). No credentials, no path.
 */
export const PROXY_URL_PATTERN = /^(?:http:\/\/[^\s/@?#]+|socks5:\/\/[^\s/@?#]+:\d+)\/?$/;

const WindowConfigSchema = strictOptions({
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
  /**
   * `false` keeps the window hidden after the first page load; the frontend
   * calls `tyndWindow.show()` when it is ready (splash-screen pattern).
   * Defaults to `true` (shown once the first page has rendered).
   */
  visible: v.optional(v.boolean()),
  /** Start in kiosk mode: borderless fullscreen pinned above other windows. */
  kiosk: v.optional(v.boolean()),
  /**
   * macOS title bar: `"visible"` (default), `"transparent"` (see-through,
   * title kept) or `"overlay"` (hidden title, page drawn under the traffic
   * lights). Ignored on Windows and Linux.
   */
  titleBarStyle: v.optional(v.picklist(["visible", "transparent", "overlay"])),
  /** Initial outer position in logical pixels; needs both `x` and `y`. `center` wins. */
  x: v.optional(v.pipe(v.number(), v.finite())),
  y: v.optional(v.pipe(v.number(), v.finite())),
  /** Theme override: `"light"`, `"dark"` or `"system"` (default, follows the OS). */
  theme: v.optional(v.picklist(["light", "dark", "system"])),
  /** Hide the window from the taskbar (Windows / Linux; ignored on macOS). */
  skipTaskbar: v.optional(v.boolean()),
  /**
   * Native background shown before the page paints and through transparent
   * HTML. Channels are `0..255`; `a` defaults to `255`.
   */
  backgroundColor: v.optional(
    strictOptions({
      r: v.pipe(v.number(), v.integer(), v.minValue(0), v.maxValue(255)),
      g: v.pipe(v.number(), v.integer(), v.minValue(0), v.maxValue(255)),
      b: v.pipe(v.number(), v.integer(), v.minValue(0), v.maxValue(255)),
      a: v.optional(v.pipe(v.number(), v.integer(), v.minValue(0), v.maxValue(255))),
    }),
  ),
  userAgent: v.optional(v.pipe(v.string(), v.minLength(1))),
  /**
   * Use a non-persistent webview data store. Cookies, cache, and local
   * storage are discarded when the window closes.
   */
  incognito: v.optional(v.boolean()),
  /**
   * Handling of `window.open()` / `target="_blank"` requests from this window.
   * Defaults to `"allow"` (platform popup, gated by the navigation policy).
   * See {@link NewWindowPolicy}.
   */
  newWindow: v.optional(v.picklist(NEW_WINDOW_POLICIES)),
  /**
   * Whether this window may download files into the OS downloads folder.
   * Unset: allowed only while the window shows app content and the download
   * URL is app-local (or a `blob:` URL of app-local content); refused
   * otherwise. Set `"allow"` to download remote files. See {@link DownloadPolicy}.
   */
  downloads: v.optional(v.picklist(DOWNLOAD_POLICIES)),
  /**
   * Route this window's traffic through a proxy: `http://host:port` or
   * `socks5://host:port`. No credentials. Unset = system networking.
   */
  proxy: v.optional(
    v.pipe(
      v.string(),
      v.regex(PROXY_URL_PATTERN, "must be http://host[:port] or socks5://host:port"),
    ),
  ),
  /**
   * Camera, microphone, geolocation... requests from pages in this window:
   * `"allow"`, `"deny"` or `"prompt"` (the WebView's own prompt; WebKitGTK
   * denies). A kind left unset prompts while the window shows app content and
   * is denied while it shows remote content. See {@link WebPermissions}.
   */
  permissions: v.optional(WebPermissionsSchema),

  // ─────────── GPU acceleration knobs (all optional, sane defaults) ───────────
  /**
   * Enable WebGPU. Defaults to `true`. Disable per-app only when the
   * workload is incompatible with waking the GPU process at all
   * (e.g. battery-sensitive utility apps). Three.js, Babylon, and any
   * WebGL-only stack keep working when `webgpu: false`.
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
  hardwareAcceleration: v.optional(v.picklist(["default", "high-performance", "low-power"])),
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

/**
 * Native OS roles for menu items. Role items are managed by the OS; `label`
 * overrides their text. `fullscreen`, `hideOthers`, `showAll`, `services`
 * and `bringAllToFront` exist on macOS only; Linux only has the edit roles
 * (`undo` … `selectAll`) and `separator`. An unavailable role renders nothing.
 */
export const MENU_ROLES = [
  "quit",
  "copy",
  "cut",
  "paste",
  "undo",
  "redo",
  "selectAll",
  "minimize",
  "maximize",
  "fullscreen",
  "close",
  "separator",
  "about",
  "hide",
  "hideOthers",
  "showAll",
  "services",
  "bringAllToFront",
] as const;
export type MenuRole = (typeof MENU_ROLES)[number];

/**
 * Platform icons for menu items (`nativeIcon`): the AppKit image on macOS,
 * the shell stock icon on Windows, the freedesktop theme icon on Linux, where
 * an equivalent exists.
 */
export const MENU_NATIVE_ICONS = [
  "add",
  "advanced",
  "bluetooth",
  "bookmarks",
  "caution",
  "colorPanel",
  "columnView",
  "computer",
  "enterFullScreen",
  "everyone",
  "exitFullScreen",
  "flowView",
  "folder",
  "folderBurnable",
  "folderSmart",
  "followLinkFreestanding",
  "fontPanel",
  "goLeft",
  "goRight",
  "home",
  "iChatTheater",
  "iconView",
  "info",
  "invalidDataFreestanding",
  "leftFacingTriangle",
  "listView",
  "lockLocked",
  "lockUnlocked",
  "menuMixedState",
  "menuOnState",
  "mobileMe",
  "multipleDocuments",
  "network",
  "path",
  "preferencesGeneral",
  "quickLook",
  "refreshFreestanding",
  "refresh",
  "remove",
  "revealFreestanding",
  "rightFacingTriangle",
  "share",
  "slideshow",
  "smartBadge",
  "statusAvailable",
  "statusNone",
  "statusPartiallyAvailable",
  "statusUnavailable",
  "stopProgressFreestanding",
  "stopProgress",
  "trashEmpty",
  "trashFull",
  "user",
  "userAccounts",
  "userGroup",
  "userGuest",
] as const;
export type MenuNativeIcon = (typeof MENU_NATIVE_ICONS)[number];

const MenuIconSchema = v.union([
  v.pipe(v.string(), v.minLength(1)),
  strictOptions({ png: v.pipe(v.string(), v.minLength(1)) }),
  strictOptions({
    rgba: v.pipe(v.string(), v.minLength(1)),
    width: v.pipe(v.number(), v.integer(), v.minValue(1), v.maxValue(512)),
    height: v.pipe(v.number(), v.integer(), v.minValue(1), v.maxValue(512)),
  }),
]);
/**
 * Menu item image: a file path (inside the fs allow-list), base64 PNG bytes
 * (`{ png }`), or base64 raw RGBA pixels (`{ rgba, width, height }`). At most
 * 512x512 px.
 */
export type MenuIcon = v.InferOutput<typeof MenuIconSchema>;

const MenuSeparatorSchema = strictOptions({ type: v.literal("separator") });
export type MenuSeparator = v.InferOutput<typeof MenuSeparatorSchema>;

const MenuActionSchema = v.pipe(
  strictOptions({
    type: v.optional(v.literal("item")),
    id: v.optional(v.string()),
    /** Required unless `role` is set (roles bring their OS label). */
    label: v.optional(v.string()),
    enabled: v.optional(v.boolean()),
    role: v.optional(v.picklist(MENU_ROLES)),
    /** Keyboard accelerator (muda format — e.g. `"CmdOrCtrl+S"`, `"Alt+F4"`). */
    accelerator: v.optional(v.string()),
    /** Image shown next to the label. Exclusive with `nativeIcon`. */
    icon: v.optional(MenuIconSchema),
    /** Platform icon shown next to the label. Exclusive with `icon`. */
    nativeIcon: v.optional(v.picklist(MENU_NATIVE_ICONS)),
  }),
  v.check(
    (item) => item.label !== undefined || item.role !== undefined,
    "menu item needs a `label` (or a `role`)",
  ),
  v.check(
    (item) => item.icon === undefined || item.nativeIcon === undefined,
    "menu item: set either `icon` or `nativeIcon`, not both",
  ),
);
export type MenuAction = v.InferOutput<typeof MenuActionSchema>;

const MenuCheckboxSchema = strictOptions({
  type: v.literal("checkbox"),
  id: v.optional(v.string()),
  label: v.string(),
  enabled: v.optional(v.boolean()),
  checked: v.optional(v.boolean()),
  accelerator: v.optional(v.string()),
});
export type MenuCheckbox = v.InferOutput<typeof MenuCheckboxSchema>;

/** Adjacent radio items in the same submenu form a group (a separator or a
 * non-radio item ends it): clicking one unchecks the others. */
const MenuRadioSchema = strictOptions({
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
  /** Handle for `menu.item(id)` / `menu.append(item, { parent: id })`. */
  id?: string | undefined;
  label: string;
  enabled?: boolean | undefined;
  icon?: MenuIcon | undefined;
  nativeIcon?: MenuNativeIcon | undefined;
  items: MenuItem[];
}
export type MenuItem = MenuSeparator | MenuAction | MenuCheckbox | MenuRadio | MenuSubmenu;

/**
 * Runtime menu entry for `tray.setMenu` and `menu.showContextMenu` (plugins
 * `@tynd/plugin-tray` and `@tynd/plugin-menu` share this wire shape).
 */
export interface TrayMenuItem {
  type?: "separator" | "submenu" | "checkbox" | "radio" | "item";
  id?: string;
  label?: string;
  enabled?: boolean;
  role?: MenuRole;
  accelerator?: string;
  checked?: boolean;
  icon?: MenuIcon;
  nativeIcon?: MenuNativeIcon;
  items?: TrayMenuItem[];
}

const MenuSubmenuSchema: v.GenericSchema<MenuSubmenu> = v.lazy(() =>
  strictOptions({
    type: v.literal("submenu"),
    id: v.optional(v.string()),
    label: v.string(),
    enabled: v.optional(v.boolean()),
    icon: v.optional(MenuIconSchema),
    nativeIcon: v.optional(v.picklist(MENU_NATIVE_ICONS)),
    items: v.array(MenuItemSchema),
  }),
);

// Dispatch on `type` (absent = "item") so an unknown key is reported against
// the item's own shape instead of a generic union mismatch.
const MENU_ITEM_SCHEMAS: Record<string, v.GenericSchema<MenuItem>> = {
  separator: MenuSeparatorSchema,
  item: MenuActionSchema,
  checkbox: MenuCheckboxSchema,
  radio: MenuRadioSchema,
  submenu: MenuSubmenuSchema,
};
// Only reached for an unrecognised `type`: reports it against the valid ones.
const MenuItemTypeSchema = v.object({ type: v.picklist(Object.keys(MENU_ITEM_SCHEMAS)) });

const MenuItemSchema: v.GenericSchema<MenuItem> = v.lazy((input) => {
  if (typeof input !== "object" || input === null) return MenuActionSchema;
  const type = (input as { type?: unknown }).type ?? "item";
  return typeof type === "string" && Object.hasOwn(MENU_ITEM_SCHEMAS, type)
    ? (MENU_ITEM_SCHEMAS[type] as v.GenericSchema<MenuItem>)
    : (MenuItemTypeSchema as unknown as v.GenericSchema<MenuItem>);
});

const TrayConfigSchema = strictOptions({
  icon: v.pipe(v.string(), v.minLength(1)),
  tooltip: v.optional(v.string()),
  menu: v.optional(v.array(MenuItemSchema)),
});
export type TrayConfig = v.InferOutput<typeof TrayConfigSchema>;

const SecurityScopeSchema = strictOptions({
  allow: v.optional(v.array(v.string())),
  deny: v.optional(v.array(v.string())),
});
export type SecurityScope = v.InferOutput<typeof SecurityScopeSchema>;

export const SecurityPolicySchema = strictOptions({
  fs: v.optional(SecurityScopeSchema),
  http: v.optional(SecurityScopeSchema),
  defaultDeny: v.optional(v.boolean()),
  allowedSensitiveCapabilities: v.optional(
    v.array(v.picklist(["audioInput", "audioOutput", "screenCapture", "window"])),
  ),
  processAllowedBinaries: v.optional(v.array(v.string())),
  shellExecAllowedCommands: v.optional(v.array(v.string())),
  /**
   * Keep the runtime grants of `capabilities.allowPath` / `allowUrl` and of
   * dialogs opened with `grantAccess: true` across restarts. Stored in the app
   * data directory; each path is resolved again on load and dropped when it no
   * longer resolves to what was granted.
   */
  persistGrants: v.optional(v.boolean()),
});
export type SecurityPolicy = v.InferOutput<typeof SecurityPolicySchema>;

const CapabilityTargetSchema = v.picklist(["linux", "macOS", "windows", "android", "iOS"]);
const CapabilityPermissionSchema = strictOptions({
  version: v.optional(v.pipe(v.number(), v.integer(), v.minValue(1))),
  identifier: v.pipe(v.string(), v.minLength(1)),
  description: v.optional(v.string()),
  commands: v.optional(
    strictOptions({
      allow: v.optional(v.array(v.pipe(v.string(), v.minLength(1)))),
      deny: v.optional(v.array(v.pipe(v.string(), v.minLength(1)))),
    }),
  ),
  scope: v.optional(v.unknown()),
  platforms: v.optional(v.array(CapabilityTargetSchema)),
});

export const CapabilitySchema = strictOptions({
  identifier: v.pipe(
    v.string(),
    v.regex(/^[a-z][a-z0-9-]{0,63}$/, "must be a kebab-case capability identifier"),
  ),
  description: v.optional(v.string()),
  remote: v.optional(
    strictOptions({
      urls: v.optional(v.array(v.pipe(v.string(), v.minLength(1)))),
    }),
  ),
  local: v.optional(v.boolean()),
  windows: v.optional(v.array(v.pipe(v.string(), v.minLength(1)))),
  webviews: v.optional(v.array(v.pipe(v.string(), v.minLength(1)))),
  permissions: v.optional(
    v.array(v.union([v.pipe(v.string(), v.minLength(1)), CapabilityPermissionSchema])),
  ),
  platforms: v.optional(v.array(CapabilityTargetSchema)),
});
export type Capability = v.InferOutput<typeof CapabilitySchema>;

/**
 * Opt-in production telemetry configured in `tynd.config.ts` (not in `app.start`).
 * Absent = disabled. A blank `dsn` / `endpoint` disables that pipeline. Custom host
 * builds compile `obs-sentry` / `obs-otel` in for the pipelines you configure.
 */
export interface ObservabilityConfig {
  sentry?: {
    dsn: string;
    environment?: string;
    release?: string;
    /** Fraction of events sent, 0..1 (default 1). */
    sampleRate?: number;
  };
  otel?: {
    endpoint: string;
    /** Defaults to the executable name. */
    serviceName?: string;
  };
}

/** `app.start({ assets })`: the `asset://` scope. */
const AssetsConfigSchema = strictOptions({
  scope: v.array(
    v.pipe(
      v.string(),
      v.check(
        (glob) => glob.startsWith("/") || /^[A-Za-z]:[\\/]/.test(glob),
        "must be an absolute path glob such as /home/me/Videos/** or C:/Users/me/Videos/**",
      ),
    ),
  ),
});
export type AssetsConfig = v.InferOutput<typeof AssetsConfigSchema>;

const AppConfigEntries = {
  window: v.optional(WindowConfigSchema),
  menu: v.optional(v.array(MenuSubmenuSchema)),
  tray: v.optional(TrayConfigSchema),
  /**
   * Host security policy installed before any WebView is created. Renderer
   * code cannot mutate this policy.
   */
  security: v.optional(SecurityPolicySchema),
  /**
   * Explicit per-window, per-webview and per-origin renderer grants.
   * Native renderer commands are denied when this is omitted or empty.
   */
  capabilities: v.optional(v.array(CapabilitySchema)),
  devUrl: v.optional(v.pipe(v.string(), v.url())),
  frontendDir: v.optional(v.pipe(v.string(), v.minLength(1))),
  /**
   * Exit the process as soon as the last window closes. Leave off (default)
   * for tray-only apps that should stay alive after all windows are hidden.
   */
  quitOnLastWindowClosed: v.optional(v.boolean()),
  /**
   * Enable the `asset://` protocol behind `convertFileSrc(path)`: globs over
   * absolute paths the webviews may load (`/home/me/Videos/**`). Absent = off;
   * `security.fs.deny` still applies.
   */
  assets: v.optional(AssetsConfigSchema),
};

/**
 * `app.start()` options. Strict at every level: an unknown key (a typo, or an
 * option that belongs elsewhere such as `plugins`, which is a `tynd.config.ts`
 * key) is rejected instead of being silently ignored at runtime.
 */
export const AppConfigSchema = strictOptions(AppConfigEntries);
export type AppConfig = v.InferOutput<typeof AppConfigSchema>;

/** One `app.start()` validation failure, with its dotted path. */
export interface ConfigIssue {
  path: string;
  message: string;
  /** The key is not an option of the object that holds it. */
  unknownKey: boolean;
}

/**
 * @internal Flattens valibot issues for `app.start()` errors. A union mismatch
 * whose candidates include an unknown key (e.g. `gpu: { vendor, id }`) is
 * reported as that key, not as a generic "Invalid type".
 */
export function configIssues(issues: readonly v.BaseIssue<unknown>[], prefix = ""): ConfigIssue[] {
  return issues.flatMap((issue) => {
    const path = [prefix, ...(issue.path?.map((p) => String(p.key)) ?? [])]
      .filter(Boolean)
      .join(".");
    const unknownKeys = issue.issues?.filter(isUnknownKey);
    if (issue.type === "union" && unknownKeys?.length) return configIssues(unknownKeys, path);
    return [{ path, message: issue.message, unknownKey: isUnknownKey(issue) }];
  });
}

function isUnknownKey(issue: v.BaseIssue<unknown>): boolean {
  return issue.type === "strict_object" && issue.expected === "never";
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
