/** Windows materials: `mica` / `tabbed` (Windows 11), `acrylic` (Windows 10 1809+), `blur`. */
export type WindowsEffect = "mica" | "tabbed" | "acrylic" | "blur";

/** macOS `NSVisualEffectView` materials. */
export type MacosEffect =
  | "titlebar"
  | "selection"
  | "menu"
  | "popover"
  | "sidebar"
  | "headerView"
  | "sheet"
  | "windowBackground"
  | "hudWindow"
  | "fullScreenUI"
  | "tooltip"
  | "contentBackground"
  | "underWindowBackground"
  | "underPageBackground";

/** Any native window material accepted by `window.setEffects`. */
export type WindowEffect = WindowsEffect | MacosEffect;

/** Options of `window.setEffects`. */
export interface WindowEffectsOptions {
  /**
   * Effects in order of preference; the first one the running OS supports is
   * applied (`["mica", "sidebar"]` gives Mica on Windows 11 and the sidebar
   * material on macOS). `[]` clears the current effect.
   */
  effects: WindowEffect[];
  /** macOS: whether the material follows the window's active state (default), or stays active / inactive. */
  state?: "followsWindowActiveState" | "active" | "inactive";
  /** macOS: corner radius of the material, in points (0-1000). */
  radius?: number;
  /** Windows `acrylic` / `blur`: tint as `"#rrggbb"` or `"#rrggbbaa"`. */
  color?: string;
}
