import type { MenuIcon, MenuNativeIcon, TrayMenuItem } from "@tynd/core";
import { bytesToBase64, currentWindowLabel, osCall, osOn } from "@tynd/core/client/bridge";

/** Config-level tray types (`app.start({ tray })`), re-exported for convenience. */
export type {
  MenuIcon,
  MenuItem,
  MenuNativeIcon,
  MenuRole,
  TrayConfig,
  TrayMenuItem,
} from "@tynd/core";

/** Id of the tray declared in `app.start({ tray })`. */
export const MAIN_TRAY = "main";

/**
 * Image accepted for the tray icon and its menu items: a file path (inside the
 * fs allow-list), PNG bytes as `{ png }`, or raw RGBA pixels as
 * `{ rgba, width, height }` (bytes or base64). Inline images are at most 512x512 px.
 */
export type TrayImage =
  | MenuIcon
  | { png: Uint8Array }
  | { rgba: Uint8Array; width: number; height: number };

/** A {@link TrayImage}, or a platform icon (`{ native: "add" }`). */
export type TrayIconSource = TrayImage | { native: MenuNativeIcon };

/** Runtime tray menu entry: the config item shape, plus byte icons. */
export interface TrayRuntimeMenuItem extends Omit<TrayMenuItem, "icon" | "items"> {
  icon?: TrayImage;
  items?: TrayRuntimeMenuItem[];
}

/** Options of {@link tray.create}. */
export interface TrayOptions {
  /** Icon image (required unless `nativeIcon` is set). */
  icon?: TrayImage;
  /** Platform icon instead of an image. */
  nativeIcon?: MenuNativeIcon;
  /** macOS: draw the icon from its alpha channel so it follows the light / dark menu bar. */
  template?: boolean;
  tooltip?: string;
  /** Text next to the icon (macOS menu bar, Linux panel). Ignored on Windows. */
  title?: string;
  menu?: TrayRuntimeMenuItem[];
  /** Open the menu on left click too (default `true`). Windows / macOS; Linux always does. */
  showMenuOnLeftClick?: boolean;
  /** Start hidden with `false`. */
  visible?: boolean;
}

/** Icon bounds in physical pixels (virtual-desktop coordinates). */
export interface TrayRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Payload of the hover events (`onEnter` / `onMove` / `onLeave`). */
export interface TrayPointerEvent {
  /** Id of the tray that fired. */
  tray: string;
  /** Cursor position, physical pixels. */
  position: { x: number; y: number };
  /** Icon bounds; `null` on Linux, which does not report them. */
  rect: TrayRect | null;
}

/** Payload of the click events. */
export interface TrayClickEvent extends TrayPointerEvent {
  button: "left" | "right" | "middle";
}

/** Payload of a tray menu item click. */
export interface TrayMenuClickEvent {
  /** Item `id`. */
  id: string;
  tray: string;
}

/** Fields of {@link TrayHandle.update}. */
export interface TrayMenuItemUpdate {
  label?: string;
  enabled?: boolean;
  checked?: boolean;
  visible?: boolean;
}

/** Placement of a window next to the tray icon. */
export interface TrayWindowPlacement {
  /** Window label. Defaults to the calling window. */
  label?: string;
  /** Pixels between the window and the taskbar / menu bar (0-200, default 8). */
  gap?: number;
}

/** Options of {@link TrayHandle.attachWindow}. */
export interface AttachWindowOptions extends TrayWindowPlacement {
  /** Hide the window when it loses focus (default `true`). */
  hideOnBlur?: boolean;
}

/** One tray icon. Every call rejects once the tray was removed. */
export interface TrayHandle {
  readonly id: string;
  /** Replace the icon: an image, or `{ native }` for a platform icon. */
  setIcon(icon: TrayIconSource, options?: { template?: boolean }): Promise<void>;
  /** macOS: draw the current icon as a template. No-op elsewhere. */
  setIconAsTemplate(template: boolean): Promise<void>;
  /** Set (or clear, pass `undefined`) the hover tooltip. */
  setTooltip(text?: string): Promise<void>;
  /** Text next to the icon (macOS menu bar, Linux panel). No-op on Windows. */
  setTitle(text?: string): Promise<void>;
  setVisible(visible: boolean): Promise<void>;
  /** Windows / macOS: open the menu on left click too. No-op on Linux. */
  setShowMenuOnLeftClick(show: boolean): Promise<void>;
  /** Replace the whole menu (`[]` clears it). A bad item leaves the old menu in place. */
  setMenu(items: TrayRuntimeMenuItem[]): Promise<void>;
  /** Icon bounds; `null` on Linux. */
  getRect(): Promise<TrayRect | null>;
  /** Windows / macOS: open the tray menu under the icon. Rejects on Linux. */
  popUpContextMenu(): Promise<void>;

  /** Change several fields of a menu item in one call. */
  update(id: string, partial: TrayMenuItemUpdate): Promise<void>;
  setItemEnabled(id: string, enabled: boolean): Promise<void>;
  setItemLabel(id: string, label: string): Promise<void>;
  /** Checkbox / radio items only; checking a radio unchecks the rest of its group. */
  setItemChecked(id: string, checked: boolean): Promise<void>;
  /** Hide or show an item without rebuilding the menu (it keeps its slot). */
  setItemVisible(id: string, visible: boolean): Promise<void>;
  /** Set or clear (`null`) a shortcut hint, e.g. `"CmdOrCtrl+Q"`. */
  setItemAccelerator(id: string, accelerator: string | null): Promise<void>;
  /** Set or clear (`null`) the icon of an item or submenu. */
  setItemIcon(id: string, icon: TrayIconSource | null): Promise<void>;
  /** Add an item (or submenu) to the menu or to submenu `parent`, at `index` (default: last). */
  append(item: TrayRuntimeMenuItem, options?: { parent?: string; index?: number }): Promise<void>;
  /** Remove a menu item (a submenu goes with its children). */
  removeItem(id: string): Promise<void>;

  /**
   * Turn an open window into the tray's popover ("menubar app"): a left click
   * on the icon shows it next to the icon (below a top bar, above the Windows
   * taskbar, inside the work area) or hides it, and it hides when it loses
   * focus. The window is hidden right away; the left click no longer opens
   * the menu. Pass a label or options; defaults to the calling window.
   * Windows and macOS only: the Linux tray reports no icon clicks, so this
   * rejects with `"tray.attachWindow: not supported on linux"` there.
   */
  attachWindow(options?: string | AttachWindowOptions): Promise<void>;
  /** Undo {@link attachWindow}; the left click opens the menu again. */
  detachWindow(): Promise<void>;
  /** Move a window next to the tray icon once (Linux: next to the pointer). */
  positionWindow(options?: string | TrayWindowPlacement): Promise<void>;

  onClick(handler: (event: TrayClickEvent) => void): () => void;
  onRightClick(handler: (event: TrayClickEvent) => void): () => void;
  onMiddleClick(handler: (event: TrayClickEvent) => void): () => void;
  /** Windows only. */
  onDoubleClick(handler: (event: TrayClickEvent) => void): () => void;
  /** Windows / macOS. */
  onEnter(handler: (event: TrayPointerEvent) => void): () => void;
  /** Windows / macOS. */
  onMove(handler: (event: TrayPointerEvent) => void): () => void;
  /** Windows / macOS. */
  onLeave(handler: (event: TrayPointerEvent) => void): () => void;
  /** A click on the item `id` of this tray's menu. */
  onMenu(id: string, handler: (event: TrayMenuClickEvent) => void): () => void;
  /** A click on any item of this tray's menu. */
  onMenuClick(handler: (event: TrayMenuClickEvent) => void): () => void;

  /** Remove the icon from the notification area. */
  remove(): Promise<void>;
}

type Wire = Record<string, unknown>;

function wireImage(image: TrayImage): MenuIcon {
  if (typeof image === "string") return image;
  if ("png" in image) {
    return { png: typeof image.png === "string" ? image.png : bytesToBase64(image.png) };
  }
  return {
    rgba: typeof image.rgba === "string" ? image.rgba : bytesToBase64(image.rgba),
    width: image.width,
    height: image.height,
  };
}

function wireIconSource(source: TrayIconSource): Wire {
  if (typeof source === "object" && "native" in source) return { nativeIcon: source.native };
  return { icon: wireImage(source) };
}

/** Wire shape: base64 icons, and an explicit `type` (the host also defaults it). */
function wireItem(item: TrayRuntimeMenuItem): Wire {
  const { icon, items, ...rest } = item;
  const out: Wire = { ...rest, type: item.type ?? (items ? "submenu" : "item") };
  if (icon !== undefined) out.icon = wireImage(icon);
  if (items) out.items = items.map(wireItem);
  return out;
}

function placement<T extends TrayWindowPlacement>(options: string | T | undefined): T {
  const resolved = (typeof options === "string" ? { label: options } : { ...options }) as T;
  resolved.label ??= currentWindowLabel();
  return resolved;
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

type TrayCall = <T = void>(method: string, args?: Wire) => Promise<T>;

type TrayEvents = Pick<
  TrayHandle,
  | "onClick"
  | "onRightClick"
  | "onMiddleClick"
  | "onDoubleClick"
  | "onEnter"
  | "onMove"
  | "onLeave"
  | "onMenu"
  | "onMenuClick"
>;

type TrayItemMethods = Pick<
  TrayHandle,
  | "update"
  | "setItemEnabled"
  | "setItemLabel"
  | "setItemChecked"
  | "setItemVisible"
  | "setItemAccelerator"
  | "setItemIcon"
  | "append"
  | "removeItem"
>;

function itemMethods(call: TrayCall): TrayItemMethods {
  return {
    update: (id, partial) => call("update", { id, partial }),
    setItemEnabled: (id, enabled) => call("setItemEnabled", { id, enabled }),
    setItemLabel: (id, label) => call("setItemLabel", { id, label }),
    setItemChecked: (id, checked) => call("setItemChecked", { id, checked }),
    setItemVisible: (id, visible) => call("setItemVisible", { id, visible }),
    setItemAccelerator: (id, accelerator) => call("setItemAccelerator", { id, accelerator }),
    setItemIcon: (id, icon) =>
      call("setItemIcon", { id, ...(icon === null ? {} : wireIconSource(icon)) }),
    append: (item, options) => call("append", { item: wireItem(item), ...options }),
    removeItem: (id) => call("removeItem", { id }),
  };
}

/** Event subscriptions, filtered to the tray `trayId`. */
function trayEvents(trayId: string): TrayEvents {
  const on = <E>(event: string, handler: (event: E) => void): (() => void) =>
    osOn(event, (raw) => {
      if (isObject(raw) && raw.tray === trayId) handler(raw as E);
    });
  const onMenuEvent = (
    filter: (itemId: string) => boolean,
    handler: (e: TrayMenuClickEvent) => void,
  ) =>
    osOn("menu:action", (raw) => {
      if (isObject(raw) && raw.tray === trayId && typeof raw.id === "string" && filter(raw.id)) {
        handler({ id: raw.id, tray: trayId });
      }
    });
  return {
    onClick: (handler) => on("tray:click", handler),
    onRightClick: (handler) => on("tray:right-click", handler),
    onMiddleClick: (handler) => on("tray:middle-click", handler),
    onDoubleClick: (handler) => on("tray:double-click", handler),
    onEnter: (handler) => on("tray:enter", handler),
    onMove: (handler) => on("tray:move", handler),
    onLeave: (handler) => on("tray:leave", handler),
    onMenu: (id, handler) => onMenuEvent((clicked) => clicked === id, handler),
    onMenuClick: (handler) => onMenuEvent(() => true, handler),
  };
}

/** Handle for the tray `id` (no host call: use `tray.get` to check it exists). */
function handle(id: string): TrayHandle {
  const call: TrayCall = (method, args = {}) => osCall("tray", method, { tray: id, ...args });
  return {
    id,
    setIcon: (icon, options) => call("setIcon", { ...wireIconSource(icon), ...options }),
    setIconAsTemplate: (template) => call("setIconAsTemplate", { template }),
    setTooltip: (text) => call("setTooltip", { text }),
    setTitle: (text) => call("setTitle", { text }),
    setVisible: (visible) => call("setVisible", { visible }),
    setShowMenuOnLeftClick: (show) => call("setShowMenuOnLeftClick", { show }),
    setMenu: (items) => call("setMenu", { items: items.map(wireItem) }),
    getRect: () => call<TrayRect | null>("getRect"),
    popUpContextMenu: () => call("popUpContextMenu"),
    ...itemMethods(call),
    attachWindow: (options) => call("attachWindow", { ...placement(options) }),
    detachWindow: () => call("detachWindow"),
    positionWindow: (options) => call("positionWindow", { ...placement(options) }),
    ...trayEvents(id),
    remove: () => call("remove"),
  };
}

const main = handle(MAIN_TRAY);

/**
 * System tray icons. The methods of `tray` itself drive the `"main"` tray
 * (declared in `app.start({ tray })` or created with `tray.create("main", …)`);
 * `create` / `get` return a {@link TrayHandle} for any other icon.
 */
export const tray = {
  ...main,
  /** Add a tray icon. Rejects if `id` (1-64 letters, digits, `_`, `-`) is taken. */
  async create(id: string, options: TrayOptions): Promise<TrayHandle> {
    const { icon, menu, ...rest } = options;
    await osCall("tray", "create", {
      tray: id,
      ...rest,
      ...(icon === undefined ? {} : { icon: wireImage(icon) }),
      ...(menu === undefined ? {} : { menu: menu.map(wireItem) }),
    });
    return handle(id);
  },
  /** Handle of an existing tray, or `null`. */
  async get(id: string): Promise<TrayHandle | null> {
    const ids = await osCall<string[]>("tray", "list");
    return ids.includes(id) ? handle(id) : null;
  },
  /** Ids of the live trays. */
  list(): Promise<string[]> {
    return osCall("tray", "list");
  },
  /** Remove a tray icon (default `"main"`). */
  remove(id: string = MAIN_TRAY): Promise<void> {
    return osCall("tray", "remove", { tray: id });
  },
};
