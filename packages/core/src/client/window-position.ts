import { tynd } from "../logger";
import type { Monitor } from "./monitor";
import type { WindowPreset, WindowSize } from "./window-types";

/**
 * Heuristic taskbar / menu-bar thickness used when the host's `Monitor` API
 * does not report a usable `workArea`. Values are in *logical* pixels.
 * Real workarea sizes vary by OS theme + DPI; these are conservative defaults
 * that keep the window from clipping under the bar in the common case. Apps
 * that need pixel-accurate tray placement should track the cursor in their
 * own `tray:click` handler instead.
 */
const TASKBAR_HEURISTIC_PX = {
  win: 40,
  macos: 25,
  linux: 32,
} as const;

function detectTaskbarHeight(): number {
  if (typeof navigator === "undefined") return TASKBAR_HEURISTIC_PX.win;
  const ua = navigator.userAgent || "";
  if (ua.includes("Mac")) return TASKBAR_HEURISTIC_PX.macos;
  if (ua.includes("Linux")) return TASKBAR_HEURISTIC_PX.linux;
  return TASKBAR_HEURISTIC_PX.win;
}

interface WindowFrame {
  left: number;
  top: number;
  width: number;
  height: number;
  windowWidth: number;
  windowHeight: number;
  taskbar: number;
}

interface Point {
  x: number;
  y: number;
}

type Positioner = (frame: WindowFrame, padding: number) => Point;

const POSITIONERS = {
  topLeft: (f, p) => ({ x: f.left + p, y: f.top + p }),
  topRight: (f, p) => ({ x: f.left + f.width - f.windowWidth - p, y: f.top + p }),
  topCenter: (f, p) => ({ x: f.left + (f.width - f.windowWidth) / 2, y: f.top + p }),
  leftCenter: (f, p) => ({ x: f.left + p, y: f.top + (f.height - f.windowHeight) / 2 }),
  rightCenter: (f, p) => ({
    x: f.left + f.width - f.windowWidth - p,
    y: f.top + (f.height - f.windowHeight) / 2,
  }),
  bottomLeft: (f, p) => ({
    x: f.left + p,
    y: f.top + f.height - f.windowHeight - p,
  }),
  bottomRight: (f, p) => ({
    x: f.left + f.width - f.windowWidth - p,
    y: f.top + f.height - f.windowHeight - p,
  }),
  bottomCenter: (f, p) => ({
    x: f.left + (f.width - f.windowWidth) / 2,
    y: f.top + f.height - f.windowHeight - p,
  }),
  center: (f) => ({
    x: f.left + (f.width - f.windowWidth) / 2,
    y: f.top + (f.height - f.windowHeight) / 2,
  }),
  trayLeft: (f) => ({ x: f.left, y: f.top + f.taskbar }),
  trayRight: (f) => ({
    x: f.left + f.width - f.windowWidth,
    y: f.top + f.taskbar,
  }),
  trayCenter: (f) => ({
    x: f.left + (f.width - f.windowWidth) / 2,
    y: f.top + f.taskbar,
  }),
  trayBottomCenter: (f) => ({
    x: f.left + (f.width - f.windowWidth) / 2,
    y: f.top + f.height - f.windowHeight - f.taskbar,
  }),
} satisfies Record<WindowPreset, Positioner>;

const TRAY_PRESETS: ReadonlySet<WindowPreset> = new Set([
  "trayLeft",
  "trayRight",
  "trayCenter",
  "trayBottomCenter",
]);

function toWindowFrame(monitor: Monitor, windowSize: WindowSize): WindowFrame {
  const scale = monitor.scale || 1;
  return {
    left: monitor.position.x / scale,
    top: monitor.position.y / scale,
    width: monitor.size.width / scale,
    height: monitor.size.height / scale,
    windowWidth: windowSize.width,
    windowHeight: windowSize.height,
    taskbar: detectTaskbarHeight(),
  };
}

function warnAboutApproximateTrayPosition(preset: WindowPreset, scale: number): void {
  if (scale === 1 || !TRAY_PRESETS.has(preset)) return;
  tynd.warn(
    `window.positionAt("${preset}"): scale=${scale} — tray placement is approximate on multi-DPI setups`,
  );
}

export async function positionWindowAt(
  call: <T>(method: string, args?: Record<string, unknown>) => Promise<T>,
  preset: WindowPreset,
  padding = 20,
): Promise<void> {
  const [monitor, windowSize] = await Promise.all([
    call<Monitor | null>("currentMonitor"),
    call<WindowSize>("getOuterSize"),
  ]);
  if (!monitor) return;

  warnAboutApproximateTrayPosition(preset, monitor.scale || 1);
  const position = POSITIONERS[preset](toWindowFrame(monitor, windowSize), padding);
  await call("setPosition", { ...position });
}
