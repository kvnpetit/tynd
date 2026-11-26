import type {
  MenuIcon,
  MenuItem,
  MenuNativeIcon,
  MenuRole,
  MenuSubmenu,
  TrayMenuItem,
} from "@tynd/core";
import { bytesToBase64, osCall, osOn } from "@tynd/core/client/bridge";

/** Config-level menu bar types (`app.start({ menu })`), re-exported for convenience. */
export type {
  MenuAction,
  MenuCheckbox,
  MenuIcon,
  MenuItem,
  MenuNativeIcon,
  MenuRadio,
  MenuRole,
  MenuSeparator,
  MenuSubmenu,
} from "@tynd/core";

/**
 * Menu item image accepted by the client: a {@link MenuIcon} (file path or
 * base64), or raw bytes — PNG bytes as `{ png }`, RGBA pixels as
 * `{ rgba, width, height }`. At most 512x512 px.
 */
export type MenuItemIcon =
  | MenuIcon
  | { png: Uint8Array }
  | { rgba: Uint8Array; width: number; height: number };

/** Runtime menu entry: same shape as the config items, plus byte icons. */
export interface RuntimeMenuItem extends Omit<TrayMenuItem, "icon" | "items"> {
  icon?: MenuItemIcon;
  items?: RuntimeMenuItem[];
}

export type ContextMenuItem = RuntimeMenuItem;

/**
 * Subset of a menu item accepted by `menu.update(id, partial)` — only the
 * fields a runtime mutation can flip in one call. Use the item handle
 * (`menu.item(id)`) for the accelerator and the icon.
 */
export interface MenuItemPartial {
  /** Visible label. Windows preserves `&` accelerator hints in the text. */
  label?: string;
  /** Whether the item is interactable. */
  enabled?: boolean;
  /** Checkmark state. Errors if the target item isn't `type: "checkbox"` / `"radio"`. */
  checked?: boolean;
  /** Detach / reinsert the item without rebuilding the menu. */
  visible?: boolean;
}

/** Payload of a menu click (menu bar, tray or context menu). */
export interface MenuClickEvent {
  /** Id of the clicked item. */
  id: string;
  /** Registered context menu the click came from (`data-tynd-contextmenu`). */
  contextMenu?: string;
  /** The element's `data-tynd-contextmenu-data`, `null` when absent. */
  data?: string | null;
}

/** Where `menu.append` puts an item. */
export interface MenuAppendOptions {
  /** Id of the submenu to add to. Omit to add a top-level submenu to the bar. */
  parent?: string;
  /** Insert at this index instead of appending. */
  index?: number;
}

/** Icon change for `item.setIcon`: an image, a platform icon, or `null` to clear. */
export type MenuIconChange = MenuItemIcon | { native: MenuNativeIcon } | null;

/** Typed handle on a menu-bar item, returned by `menu.item(id)`. */
export interface MenuItemHandle {
  readonly id: string;
  /** Batch several changes in one call. */
  update(partial: MenuItemPartial): Promise<void>;
  setEnabled(enabled: boolean): Promise<void>;
  setLabel(label: string): Promise<void>;
  /** Checkbox / radio only. Checking a radio unchecks the rest of its group. */
  setChecked(checked: boolean): Promise<void>;
  setVisible(visible: boolean): Promise<void>;
  /** `null` removes the shortcut. Rejects on an invalid accelerator. */
  setAccelerator(accelerator: string | null): Promise<void>;
  /** Regular items and submenus only. */
  setIcon(icon: MenuIconChange): Promise<void>;
  /** Detach the item (and, for a submenu, everything inside it). */
  remove(): Promise<void>;
  /** Add `item` to this submenu. */
  append(item: RuntimeMenuItem, options?: { index?: number }): Promise<void>;
  onClick(handler: (event: MenuClickEvent) => void): () => void;
}

/** Target OS of {@link menu.defaultAppMenu}. */
export type MenuPlatform = "macos" | "windows" | "linux";

export interface DefaultAppMenuOptions {
  /** Title of the macOS application menu. Defaults to `"App"` (macOS shows the bundle name). */
  appName?: string;
  /** Defaults to the current OS. */
  platform?: MenuPlatform;
}

type WireIcon = MenuIcon;

function wireIcon(icon: MenuItemIcon): WireIcon {
  if (typeof icon === "string") return icon;
  if ("png" in icon) {
    return { png: typeof icon.png === "string" ? icon.png : bytesToBase64(icon.png) };
  }
  return {
    rgba: typeof icon.rgba === "string" ? icon.rgba : bytesToBase64(icon.rgba),
    width: icon.width,
    height: icon.height,
  };
}

/** Wire shape: every entry carries an explicit `type` and base64 icons. */
function wireItem(entry: RuntimeMenuItem | MenuItem): Record<string, unknown> {
  const item = entry as RuntimeMenuItem;
  const { icon, items, ...rest } = item;
  const type = item.type ?? (items ? "submenu" : "item");
  const out: Record<string, unknown> = { ...rest, type };
  if (icon !== undefined) out.icon = wireIcon(icon);
  if (items) out.items = items.map(wireItem);
  return out;
}

function wireItems(items: (RuntimeMenuItem | MenuItem)[]): Record<string, unknown>[] {
  return items.map(wireItem);
}

function currentPlatform(): MenuPlatform {
  const platform = (globalThis as { process?: { platform?: string } }).process?.platform;
  if (platform === "darwin") return "macos";
  if (platform === "win32") return "windows";
  if (platform) return "linux";
  const agent = (globalThis as { navigator?: { userAgent?: string } }).navigator?.userAgent ?? "";
  if (/Mac OS X|Macintosh/.test(agent)) return "macos";
  if (/Windows/.test(agent)) return "windows";
  return "linux";
}

function role(r: MenuRole): MenuItem {
  return { type: "item", role: r };
}

const SEPARATOR: MenuItem = { type: "separator" };

function editMenu(): MenuSubmenu {
  return {
    type: "submenu",
    id: "edit",
    label: "Edit",
    items: [
      role("undo"),
      role("redo"),
      SEPARATOR,
      role("cut"),
      role("copy"),
      role("paste"),
      role("selectAll"),
    ],
  };
}

function setChecked(id: string, checked: boolean): Promise<void> {
  return osCall("menu", "setItemChecked", { id, checked });
}

/**
 * Native menus: click events, runtime mutations of the application menu bar
 * (declared in `app.start({ menu })` or set with `setMenu`), popup and
 * declarative context menus. The bar, tray and context menus share one `id`
 * namespace for click events.
 */
export const menu = {
  /** Fire `handler` when the user clicks the menu item with the given `id`. */
  onClick(id: string, handler: (event: MenuClickEvent) => void): () => void {
    return osOn("menu:action", (data: unknown) => {
      if (typeof data !== "object" || data === null) return;
      const event = data as MenuClickEvent;
      if (event.id === id) handler(event);
    });
  },
  /** Fire `handler` on every menu click (any id). */
  onAction(handler: (event: MenuClickEvent) => void): () => void {
    return osOn("menu:action", (data: unknown) => {
      if (typeof data !== "object" || data === null) return;
      if (typeof (data as MenuClickEvent).id === "string") handler(data as MenuClickEvent);
    });
  },
  /**
   * Show a native popup menu anchored on the current window. Clicks emit
   * the same `menu:action` event as the menu bar / tray — discriminate by
   * `id`. `x` / `y` are window-relative logical pixels; omitting both uses
   * the cursor position.
   */
  showContextMenu(items: ContextMenuItem[], position?: { x: number; y: number }): Promise<void> {
    return osCall("window", "showContextMenu", {
      items: wireItems(items),
      x: position?.x,
      y: position?.y,
    });
  },

  /**
   * Register a named context menu. Any element with
   * `data-tynd-contextmenu="<id>"` then opens it on right-click; the click
   * event carries `contextMenu: id` and the element's
   * `data-tynd-contextmenu-data`. Registering an existing id replaces it.
   * @example
   *   await menu.registerContextMenu("row", [{ id: "row.delete", label: "Delete" }]);
   *   // <li data-tynd-contextmenu="row" data-tynd-contextmenu-data="42">…</li>
   *   menu.onClick("row.delete", (e) => deleteRow(e.data));
   */
  registerContextMenu(id: string, items: ContextMenuItem[]): Promise<void> {
    return osCall("menu", "registerContextMenu", { id, items: wireItems(items) });
  },
  /** Forget a registered context menu. Unknown ids are ignored. */
  unregisterContextMenu(id: string): Promise<void> {
    return osCall("menu", "unregisterContextMenu", { id });
  },
  /**
   * Open a registered context menu at the cursor (or at window-relative
   * `x` / `y`). `data` rides on the click event like
   * `data-tynd-contextmenu-data`.
   */
  popupContextMenu(id: string, options?: { data?: string; x?: number; y?: number }): Promise<void> {
    return osCall("menu", "popupContextMenu", {
      menu: id,
      data: options?.data ?? null,
      x: options?.x,
      y: options?.y,
    });
  },
  /**
   * Enable or disable the webview's own context menu (Reload, Inspect…) for
   * the whole page. Elements can still opt in with
   * `data-tynd-contextmenu="default"` or open a registered menu.
   */
  setDefaultContextMenu(enabled: boolean): void {
    const root = document.documentElement;
    if (enabled) {
      if (root.getAttribute("data-tynd-contextmenu") === "none") {
        root.removeAttribute("data-tynd-contextmenu");
      }
    } else {
      root.setAttribute("data-tynd-contextmenu", "none");
    }
  },

  /** Typed handle on the menu-bar item `id`. Calls reject if the id is unknown. */
  item(id: string): MenuItemHandle {
    return {
      id,
      update: (partial) => menu.update(id, partial),
      setEnabled: (enabled) => menu.setItemEnabled(id, enabled),
      setLabel: (label) => menu.setItemLabel(id, label),
      setChecked: (checked) => setChecked(id, checked),
      setVisible: (visible) => menu.setItemVisible(id, visible),
      setAccelerator: (accelerator) => osCall("menu", "setItemAccelerator", { id, accelerator }),
      setIcon: (icon) => {
        if (icon !== null && typeof icon === "object" && "native" in icon) {
          return osCall("menu", "setItemIcon", { id, icon: null, nativeIcon: icon.native });
        }
        return osCall("menu", "setItemIcon", {
          id,
          icon: icon === null ? null : wireIcon(icon),
          nativeIcon: null,
        });
      },
      remove: () => menu.remove(id),
      append: (item, options) => menu.append(item, { parent: id, index: options?.index }),
      onClick: (handler) => menu.onClick(id, handler),
    };
  },
  /**
   * Add an item to a submenu (`parent`), or a submenu to the bar itself,
   * optionally at `index`. Rejects on a duplicate id or an out-of-range index.
   * @example
   *   await menu.append({ id: "file.recent", label: "Recent" }, { parent: "file", index: 2 });
   */
  append(item: RuntimeMenuItem, options?: MenuAppendOptions): Promise<void> {
    return osCall("menu", "append", {
      item: wireItem(item),
      parent: options?.parent ?? null,
      index: options?.index ?? null,
    });
  },
  /** Remove a menu-bar item (a submenu takes its children with it). */
  remove(id: string): Promise<void> {
    return osCall("menu", "remove", { id });
  },
  /**
   * Replace the whole application menu bar. Top-level entries must be
   * submenus. Works even when the app started without a `menu`.
   * @example
   *   await menu.setMenu(menu.defaultAppMenu({ appName: "Notes" }));
   */
  setMenu(items: (RuntimeMenuItem | MenuItem)[]): Promise<void> {
    return osCall("menu", "setMenu", { items: wireItems(items) });
  },
  /**
   * The standard menu bar of the OS, built from native roles: on macOS the
   * app menu (About, Services, Hide, Quit), Edit, View (Full Screen) and
   * Window; on Windows File (Quit), Edit and Window; on Linux Edit (GTK has
   * no native window roles). Usable in `app.start({ menu })` and `setMenu`,
   * and easy to extend before passing it on.
   */
  defaultAppMenu(options?: DefaultAppMenuOptions): MenuSubmenu[] {
    const platform = options?.platform ?? currentPlatform();
    if (platform === "macos") {
      return [
        {
          type: "submenu",
          id: "app",
          label: options?.appName ?? "App",
          items: [
            role("about"),
            SEPARATOR,
            role("services"),
            SEPARATOR,
            role("hide"),
            role("hideOthers"),
            role("showAll"),
            SEPARATOR,
            role("quit"),
          ],
        },
        { type: "submenu", id: "file", label: "File", items: [role("close")] },
        editMenu(),
        { type: "submenu", id: "view", label: "View", items: [role("fullscreen")] },
        {
          type: "submenu",
          id: "window",
          label: "Window",
          items: [role("minimize"), role("maximize"), SEPARATOR, role("bringAllToFront")],
        },
      ];
    }
    if (platform === "windows") {
      return [
        { type: "submenu", id: "file", label: "File", items: [role("quit")] },
        editMenu(),
        {
          type: "submenu",
          id: "window",
          label: "Window",
          items: [role("minimize"), role("maximize"), SEPARATOR, role("close")],
        },
      ];
    }
    return [editMenu()];
  },

  /**
   * Mutate an existing menu-bar item by `id`. Pass only the fields you want
   * to change — everything else stays as configured.
   * Rejects with a clear error if the id was never registered, and (for
   * `checked`) if the target item isn't a checkbox/radio.
   * @example
   *   await menu.update("file.save", { label: "Save As...", enabled: false });
   */
  update(id: string, partial: MenuItemPartial): Promise<void> {
    return osCall("menu", "update", { id, partial });
  },
  /** Convenience wrapper — equivalent to `update(id, { enabled })`. */
  setItemEnabled(id: string, enabled: boolean): Promise<void> {
    return osCall("menu", "setItemEnabled", { id, enabled });
  },
  /** Convenience wrapper — equivalent to `update(id, { label })`. */
  setItemLabel(id: string, label: string): Promise<void> {
    return osCall("menu", "setItemLabel", { id, label });
  },
  /**
   * Convenience wrapper — equivalent to `update(id, { checked })`. Errors
   * if the target item isn't a checkbox/radio.
   */
  setItemChecked(id: string, checked: boolean): Promise<void> {
    return setChecked(id, checked);
  },
  /**
   * Convenience wrapper — equivalent to `update(id, { visible })`. Hidden
   * items are detached and re-inserted at (close to) their original
   * position when shown again.
   */
  setItemVisible(id: string, visible: boolean): Promise<void> {
    return osCall("menu", "setItemVisible", { id, visible });
  },
};
