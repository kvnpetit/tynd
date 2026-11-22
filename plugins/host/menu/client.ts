import type { TrayMenuItem } from "@tynd/core";
import { osCall, osOn } from "@tynd/core/client/bridge";

/** Config-level menu bar types (`app.start({ menu })`), re-exported for convenience. */
export type {
  MenuAction,
  MenuCheckbox,
  MenuItem,
  MenuRadio,
  MenuRole,
  MenuSeparator,
  MenuSubmenu,
} from "@tynd/core";

export type ContextMenuItem = TrayMenuItem;

/**
 * Subset of `TrayMenuItem` accepted by `menu.update(id, partial)` — only
 * the fields a runtime mutation can actually flip. Notably excludes
 * `id` (immutable handle), `type` (would need a rebuild), `role` /
 * `accelerator` (set at build time on the OS side), and `items`
 * (submenu structure is fixed by the bar config).
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

/**
 * Menu event subscriptions + runtime mutations. Declarative menu structure
 * still lives in `tynd.config.ts` — this module lets you react to clicks
 * (both app menu bar and tray menu, which share the same `id` namespace)
 * and mutate items by id without rebuilding the whole bar.
 */
export const menu = {
  /** Fire `handler` when the user clicks the menu item with the given `id`. */
  onClick(id: string, handler: () => void): () => void {
    return osOn("menu:action", (data: unknown) => {
      if (typeof data !== "object" || data === null) return;
      if ((data as Record<string, unknown>).id === id) handler();
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
      items,
      x: position?.x,
      y: position?.y,
    });
  },

  /**
   * Mutate an existing menu-bar item by `id`. Pass only the fields you want
   * to change — everything else stays as configured in `tynd.config.ts`.
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
    return osCall("menu", "setItemChecked", { id, checked });
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
