import type { WindowConfig } from "../types";

/** Anchor preset accepted by `tyndWindow.positionAt`. */
export type WindowPreset =
  | "topLeft"
  | "topRight"
  | "topCenter"
  | "leftCenter"
  | "rightCenter"
  | "bottomLeft"
  | "bottomRight"
  | "bottomCenter"
  | "center"
  | "trayLeft"
  | "trayRight"
  | "trayCenter"
  | "trayBottomCenter";

/** Logical-pixel size pair. */
export interface WindowSize {
  width: number;
  height: number;
}

/** Logical-pixel position pair. */
export interface WindowPosition {
  x: number;
  y: number;
}

/**
 * Band for `setAlwaysOnTop(true, { level })`. macOS maps them to the
 * floating, status and screen-saver window levels (`"screen-saver"` stays above
 * fullscreen apps); Windows and Linux have a single topmost band.
 */
export type AlwaysOnTopLevel = "floating" | "status" | "screen-saver";

/** How insistently `requestAttention` asks for the user's attention. */
export type AttentionKind = "informational" | "critical";

/** Event passed to `onCloseRequested` handlers. */
export interface CloseRequestedEvent {
  /** Cancel the pending close. Must be called synchronously inside the handler. */
  preventDefault(): void;
}

// Defined once next to the window config schema; re-exported for the window client.
export type { DownloadPolicy, NewWindowPolicy } from "../types";

/**
 * Native-window options shared by `app.start({ window })` and
 * `multiwindow.create()`. The GPU knobs are app-wide (one GPU process) and
 * are set on the primary window only.
 */
export type WindowOptions = Omit<
  WindowConfig,
  | "webgpu"
  | "hardwareAcceleration"
  | "preferredFps"
  | "contextLostRecovery"
  | "colorSpace"
  | "hdr"
  | "vrr"
  | "gpu"
>;

/** Event passed to `onNavigation` handlers. */
export interface NavigationEvent extends WindowEventBase {
  url: string;
  /** `false` when the security policy blocked this navigation. */
  allowed: boolean;
}

/** Event passed to `onPageLoad` handlers. */
export interface PageLoadEvent extends WindowEventBase {
  phase: "started" | "finished";
  url: string;
}

/** Common envelope for every window-scoped event. */
export interface WindowEventBase {
  label: string;
}

/** Event payload for `onDragDrop` handlers. */
export interface DragDropEvent extends WindowEventBase {
  /** Native OS file paths. */
  paths: string[];
  /** Cursor position relative to the webview top-left, logical pixels. */
  x: number;
  y: number;
}

/** Event payload for `onDragOver` handlers. */
export interface DragOverEvent extends WindowEventBase {
  x: number;
  y: number;
}

/** CSS-style cursor icon names accepted by `tyndWindow.setCursorIcon`. */
export type CursorIcon =
  | "default"
  | "crosshair"
  | "hand"
  | "arrow"
  | "move"
  | "text"
  | "wait"
  | "help"
  | "progress"
  | "notAllowed"
  | "contextMenu"
  | "cell"
  | "verticalText"
  | "alias"
  | "copy"
  | "noDrop"
  | "grab"
  | "grabbing"
  | "allScroll"
  | "zoomIn"
  | "zoomOut"
  | "eResize"
  | "nResize"
  | "neResize"
  | "nwResize"
  | "sResize"
  | "seResize"
  | "swResize"
  | "wResize"
  | "ewResize"
  | "nsResize"
  | "neswResize"
  | "nwseResize"
  | "colResize"
  | "rowResize";

/** Edge/corner identifier accepted by `tyndWindow.startResizeDrag`. */
export type ResizeDirection =
  | "east"
  | "north"
  | "northEast"
  | "northWest"
  | "south"
  | "southEast"
  | "southWest"
  | "west";
