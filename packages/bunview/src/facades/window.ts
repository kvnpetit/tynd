import type { WebviewHost } from "../host";
import type { WindowPosition } from "../types";

export function windowFacade(host: WebviewHost | undefined) {
  return {
    setTitle:       (title: string)        => host?.setTitle(title),
    setSize:        (w: number, h: number) => host?.setSize(w, h),
    navigate:       (url: string)          => host?.navigate(url),
    close:          ()                     => host?.close(),
    minimize:       ()                     => host?.minimize(),
    maximize:       ()                     => host?.maximize(),
    restore:        ()                     => host?.restore(),
    fullscreen:     (enter = true)         => host?.fullscreen(enter),
    center:         ()                     => host?.center(),
    setMinSize:     (w: number, h: number) => host?.setMinSize(w, h),
    setMaxSize:     (w: number, h: number) => host?.setMaxSize(w, h),
    setAlwaysOnTop: (on: boolean)          => host?.setAlwaysOnTop(on),
    hide:           ()                     => host?.hide(),
    show:           ()                     => host?.show(),
    setPosition:    (x: number, y: number) => host?.setPosition(x, y),
    getPosition:    ()                     => host?.getPosition() ?? Promise.resolve({ x: 0, y: 0, width: 0, height: 0 }),
    setVibrancy:    (effect: string)       => host?.setVibrancy(effect),
    setButtons:     (opts: { minimize?: boolean; maximize?: boolean; close?: boolean }) =>
      host?.setButtons(opts.minimize ?? true, opts.maximize ?? true, opts.close ?? true),
    position:       (pos: WindowPosition, monitor?: number) => host?.positionWindow(pos, monitor),
    getMonitors:    () => host?.getMonitors() ?? Promise.resolve([]),
    onMoved:        (cb: (pos: { x: number; y: number }) => void)            => { host?.on("windowMoved", cb); },
    onResized:      (cb: (size: { width: number; height: number }) => void)  => { host?.on("windowResized", cb); },
    onFocusChanged: (cb: (focused: boolean) => void)                         => { host?.on("windowFocusChanged", cb); },
    focus:               ()                                                  => host?.focus(),
    setEnabled:          (enabled: boolean)                                  => host?.setEnabled(enabled),
    setDecorations:      (decorated: boolean)                                => host?.setDecorations(decorated),
    setShadow:           (shadow: boolean)                                   => host?.setShadow(shadow),
    setBackgroundColor:  (r: number, g: number, b: number, a?: number)       => host?.setBackgroundColor(r, g, b, a),
    setTitleBarStyle:    (style: "visible" | "transparent" | "overlay" | "hidden") => host?.setTitleBarStyle(style),
    setSkipTaskbar:      (skip: boolean)           => host?.setSkipTaskbar(skip),
    requestUserAttention:(critical?: boolean)      => host?.requestUserAttention(critical ?? false),
    getTheme:            ()                        => host?.getTheme() ?? Promise.resolve("light" as const),
    setContentProtected: (protect: boolean)        => host?.setContentProtected(protect),
    setAlwaysOnBottom:   (on: boolean)             => host?.setAlwaysOnBottom(on),
    setProgressBar:      (progress: number | null) => host?.setProgressBar(progress),
    setBadgeCount:       (count: number | null)    => host?.setBadgeCount(count),
  };
}
