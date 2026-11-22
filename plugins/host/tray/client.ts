import type { TrayMenuItem } from "@tynd/core";
import { osCall, osOn } from "@tynd/core/client/bridge";

/** Config-level tray types (`app.start({ tray })`), re-exported for convenience. */
export type { MenuItem, MenuRole, TrayConfig, TrayMenuItem } from "@tynd/core";

/** Cursor position carried by tray hover events, in physical pixels. */
type TrayPointer = { x: number; y: number };

/** Frontend bindings for the host's `tray` API (icon, tooltip, menus, click events). */
export const tray = {
  /** Replace the tray icon. Accepts the same image formats as the config. */
  setIcon(path: string): Promise<void> {
    return osCall("tray", "setIcon", { path });
  },
  /** Set (or clear, pass `undefined`) the hover tooltip. */
  setTooltip(text?: string): Promise<void> {
    return osCall("tray", "setTooltip", { text });
  },
  /** macOS: text next to the icon in the menu bar. No-op elsewhere. */
  setTitle(text?: string): Promise<void> {
    return osCall("tray", "setTitle", { text });
  },
  setVisible(visible: boolean): Promise<void> {
    return osCall("tray", "setVisible", { visible });
  },
  /**
   * macOS: render the tray icon as a template so it adapts to light / dark
   * menu bar automatically. No-op on Windows / Linux.
   */
  setIconAsTemplate(template: boolean): Promise<void> {
    return osCall("tray", "setIconAsTemplate", { template });
  },
  /** Replace the entire tray menu. Pass `[]` to clear. */
  setMenu(items: TrayMenuItem[]): Promise<void> {
    return osCall("tray", "setMenu", { items });
  },

  /**
   * Enable or disable an existing tray menu item by `id`. Reaches into the
   * native menu directly — much cheaper than rebuilding via `setMenu` when
   * you just need to grey out a single action. Rejects with a clear error
   * if the id was never registered.
   */
  setItemEnabled(id: string, enabled: boolean): Promise<void> {
    return osCall("tray", "setItemEnabled", { id, enabled });
  },
  /**
   * Change the visible label of an existing tray menu item by `id`. Note
   * that platform-specific accelerator hints (e.g. Windows `&File`) are
   * preserved — pass the same prefix in `label` if you want it shown.
   */
  setItemLabel(id: string, label: string): Promise<void> {
    return osCall("tray", "setItemLabel", { id, label });
  },
  /**
   * Toggle the checkmark on a `type: "checkbox"` (or `"radio"`) item by
   * `id`. Errors if the target item isn't a checkbox/radio — for plain
   * action items the checkmark glyph isn't part of the UI.
   */
  setItemChecked(id: string, checked: boolean): Promise<void> {
    return osCall("tray", "setItemChecked", { id, checked });
  },
  /**
   * Show or hide an item without rebuilding the menu. Hidden items are
   * detached from their parent menu and re-inserted at (close to) their
   * original position when shown again — siblings hidden in the meantime
   * are taken into account so order stays sane.
   */
  setItemVisible(id: string, visible: boolean): Promise<void> {
    return osCall("tray", "setItemVisible", { id, visible });
  },

  onClick(handler: () => void): () => void {
    return osOn("tray:click", () => handler());
  },
  onRightClick(handler: () => void): () => void {
    return osOn("tray:right-click", () => handler());
  },
  onDoubleClick(handler: () => void): () => void {
    return osOn("tray:double-click", () => handler());
  },
  /** Mouse entered the tray icon's bounds. `x`/`y` are physical pixels. */
  onEnter(handler: (pos: TrayPointer) => void): () => void {
    return osOn("tray:enter", (raw) => handler(raw as TrayPointer));
  },
  /** Mouse moved over the tray icon. */
  onMove(handler: (pos: TrayPointer) => void): () => void {
    return osOn("tray:move", (raw) => handler(raw as TrayPointer));
  },
  /** Mouse left the tray icon's bounds. */
  onLeave(handler: (pos: TrayPointer) => void): () => void {
    return osOn("tray:leave", (raw) => handler(raw as TrayPointer));
  },
  /** Fire `handler` when the tray menu item with the given `id` is clicked (same as `menu.onClick`). */
  onMenu(id: string, handler: () => void): () => void {
    return osOn("menu:action", (data: unknown) => {
      if (typeof data !== "object" || data === null) return;
      if ((data as Record<string, unknown>).id === id) handler();
    });
  },
};
