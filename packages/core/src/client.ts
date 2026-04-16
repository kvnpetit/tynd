/**
 * @vorn/core/client — frontend API
 * Import from "@vorn/core/client" in your frontend code only (browser context).
 * For backend use: import from "@vorn/core"
 */

import type {
  ConfirmOptions,
  Emitter,
  EmitterMap,
  MessageOptions,
  NotificationOptions,
  OpenFileOptions,
  SaveFileOptions,
} from "./types.js"

export type {
  ConfirmOptions,
  Emitter,
  EmitterMap,
  MessageOptions,
  NotificationOptions,
  OpenFileOptions,
  SaveFileOptions,
}

import { vorn } from "./logger.js"

// ── Type utilities ────────────────────────────────────────────────────────────

type UnionToIntersection<U> = (U extends unknown ? (k: U) => void : never) extends (
  k: infer I,
) => void
  ? I
  : never

/** Extract the merged event map from all Emitter exports in a backend module */
type ModuleEvents<T> = UnionToIntersection<
  { [K in keyof T]: T[K] extends Emitter<infer E> ? E : never }[keyof T]
>

/** Map exported functions to their async proxy equivalents */
type ModuleFunctions<T> = {
  [K in keyof T as T[K] extends (...args: infer _A) => infer _R ? K : never]: T[K] extends (
    ...args: infer A
  ) => infer R
    ? (...args: A) => Promise<Awaited<R>>
    : never
}

/** The fully-typed client returned by createBackend<T>() */
export type BackendClient<T> = ModuleFunctions<T> & {
  /**
   * Subscribe to a backend event. Returns an unsubscribe function.
   * Event names and payload types are inferred from exported emitters.
   *
   * @example
   * api.on("userCreated", (user) => console.log(user.name))
   */
  on<K extends keyof ModuleEvents<T> & string>(
    event: K,
    handler: (payload: ModuleEvents<T>[K]) => void,
  ): () => void

  /**
   * Subscribe to a backend event once. Auto-unsubscribes after first call.
   */
  once<K extends keyof ModuleEvents<T> & string>(
    event: K,
    handler: (payload: ModuleEvents<T>[K]) => void,
  ): () => void
}

// ── Runtime types ─────────────────────────────────────────────────────────────

declare global {
  interface Window {
    __vorn__: {
      call(fn: string, args: unknown[]): Promise<unknown>
      os_call(api: string, method: string, args: unknown): Promise<unknown>
      os_on(name: string, handler: (data: unknown) => void): () => void
      on(name: string, handler: (payload: unknown) => void): () => void
      off(name: string, handler: (payload: unknown) => void): void
    }
    __vorn_os_result__: (id: string, ok: boolean, value: unknown) => void
    __vorn_os_event__: (name: string, data: unknown) => void
  }
}

// ── createBackend ─────────────────────────────────────────────────────────────

/**
 * Create a fully type-safe proxy to your backend.
 * Import the backend module type only — zero runtime overhead from types.
 *
 * @example
 * import { createBackend } from "@vorn/core/client"
 * import type * as backend from "../backend/main"
 *
 * const api = createBackend<typeof backend>()
 *
 * const msg = await api.greet("Alice")         // string ✅
 * api.on("userCreated", (user) => { ... })     // typed ✅
 */
export function createBackend<T>(): BackendClient<T> {
  return new Proxy({} as BackendClient<T>, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return undefined
      // Guard against Promise resolution probes — if the Proxy were thenable,
      // `await api` would call `api.then(...)` as a backend function.
      if (prop === "then" || prop === "catch" || prop === "finally") return undefined

      if (!window.__vorn__) {
        vorn.error("window.__vorn__ is not available — are you running outside a vorn app?")
        return () => Promise.reject(new Error("[vorn] not in a vorn app context"))
      }

      // Event subscription helpers
      if (prop === "on") {
        return (event: string, handler: (p: unknown) => void) => window.__vorn__.on(event, handler)
      }

      if (prop === "once") {
        return (event: string, handler: (p: unknown) => void) => {
          let called = false
          const wrapper = (p: unknown) => {
            if (!called) {
              called = true
              window.__vorn__.off(event, wrapper)
              handler(p)
            }
          }
          window.__vorn__.on(event, wrapper)
          return () => window.__vorn__.off(event, wrapper)
        }
      }

      // Default: proxy as a backend function call
      return (...args: unknown[]) => window.__vorn__.call(prop, args)
    },
  })
}

// ── OS API internal helper ────────────────────────────────────────────────────

function _osCall<T>(api: string, method: string, args: unknown = null): Promise<T> {
  return window.__vorn__.os_call(api, method, args) as Promise<T>
}

// ── dialog ────────────────────────────────────────────────────────────────────

/**
 * Native file system and message dialogs.
 *
 * @example
 * const file = await dialog.openFile({ title: "Open image", filters: [{ name: "Images", extensions: ["png", "jpg"] }] })
 * const ok = await dialog.confirm("Delete this file?")
 */
export const dialog = {
  /** Open a single-file picker. Returns the chosen path, or null if cancelled. */
  openFile(opts?: OpenFileOptions): Promise<string | null> {
    return _osCall("dialog", "openFile", opts ?? null)
  },

  /** Open a multi-file picker. Returns an array of paths, or null if cancelled. */
  openFiles(opts?: OpenFileOptions): Promise<string[] | null> {
    return _osCall("dialog", "openFiles", opts ?? null)
  },

  /** Open a save-file dialog. Returns the chosen path, or null if cancelled. */
  saveFile(opts?: SaveFileOptions): Promise<string | null> {
    return _osCall("dialog", "saveFile", opts ?? null)
  },

  /** Show a native message box. */
  message(message: string, opts?: MessageOptions): Promise<void> {
    return _osCall("dialog", "message", { message, ...opts })
  },

  /** Show a native OK/Cancel confirm dialog. Returns true if OK was clicked. */
  confirm(message: string, opts?: ConfirmOptions): Promise<boolean> {
    return _osCall("dialog", "confirm", { message, ...opts })
  },
}

// ── vornWindow ────────────────────────────────────────────────────────────────

/**
 * Control the native application window from the frontend.
 *
 * @example
 * await vornWindow.setTitle("My App — Unsaved")
 * await vornWindow.maximize()
 */
export const vornWindow = {
  setTitle(title: string): Promise<void> {
    return _osCall("window", "setTitle", { title })
  },
  setSize(width: number, height: number): Promise<void> {
    return _osCall("window", "setSize", { width, height })
  },
  minimize(): Promise<void> {
    return _osCall("window", "minimize")
  },
  unminimize(): Promise<void> {
    return _osCall("window", "unminimize")
  },
  maximize(): Promise<void> {
    return _osCall("window", "maximize")
  },
  unmaximize(): Promise<void> {
    return _osCall("window", "unmaximize")
  },
  center(): Promise<void> {
    return _osCall("window", "center")
  },
  show(): Promise<void> {
    return _osCall("window", "show")
  },
  hide(): Promise<void> {
    return _osCall("window", "hide")
  },
  setFullscreen(fullscreen: boolean): Promise<void> {
    return _osCall("window", "setFullscreen", { fullscreen })
  },
  setAlwaysOnTop(always: boolean): Promise<void> {
    return _osCall("window", "setAlwaysOnTop", { always })
  },
  setDecorations(decorations: boolean): Promise<void> {
    return _osCall("window", "setDecorations", { decorations })
  },
  isMaximized(): Promise<boolean> {
    return _osCall("window", "isMaximized")
  },
  isMinimized(): Promise<boolean> {
    return _osCall("window", "isMinimized")
  },
  isFullscreen(): Promise<boolean> {
    return _osCall("window", "isFullscreen")
  },
  isVisible(): Promise<boolean> {
    return _osCall("window", "isVisible")
  },

  /**
   * Subscribe to a native menu bar item click by its `id`.
   * Returns an unsubscribe function.
   *
   * @example
   * vornWindow.onMenu("file.open", () => openFile())
   */
  onMenu(id: string, handler: () => void): () => void {
    return window.__vorn__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.id as string) === id) handler()
    })
  },
}

// ── clipboard ─────────────────────────────────────────────────────────────────

/**
 * Read and write the system clipboard.
 *
 * @example
 * await clipboard.writeText("Hello!")
 * const text = await clipboard.readText()
 */
export const clipboard = {
  readText(): Promise<string> {
    return _osCall("clipboard", "readText")
  },
  writeText(text: string): Promise<void> {
    return _osCall("clipboard", "writeText", text)
  },
}

// ── shell ─────────────────────────────────────────────────────────────────────

/**
 * Open URLs and paths with the system default application.
 *
 * @example
 * await shell.openExternal("https://example.com")
 * await shell.openPath("/home/user/document.pdf")
 */
export const shell = {
  openExternal(url: string): Promise<void> {
    return _osCall("shell", "openExternal", url)
  },
  openPath(path: string): Promise<void> {
    return _osCall("shell", "openPath", path)
  },
}

// ── notification ──────────────────────────────────────────────────────────────

/**
 * Send a native OS desktop notification.
 *
 * @example
 * await notification.send("Build Complete", { body: "0 errors, 0 warnings." })
 */
export const notification = {
  send(title: string, opts?: NotificationOptions): Promise<void> {
    return _osCall("notification", "send", { title, body: opts?.body ?? "" })
  },
}

// ── tray ──────────────────────────────────────────────────────────────────────

/**
 * Subscribe to system tray events.
 * The tray itself is configured in `app.start({ tray: { ... } })`.
 *
 * @example
 * tray.onClick(() => vornWindow.show())
 * tray.onMenu("show", () => vornWindow.show())
 */
export const tray = {
  /** Fires when the tray icon is left-clicked. */
  onClick(handler: () => void): () => void {
    return window.__vorn__.os_on("tray:click", () => handler())
  },

  /** Fires when the tray icon is right-clicked. */
  onRightClick(handler: () => void): () => void {
    return window.__vorn__.os_on("tray:right-click", () => handler())
  },

  /** Fires when the tray icon is double-clicked. */
  onDoubleClick(handler: () => void): () => void {
    return window.__vorn__.os_on("tray:double-click", () => handler())
  },

  /**
   * Subscribe to a tray context menu item click by its `id`.
   * Returns an unsubscribe function.
   *
   * @example
   * tray.onMenu("quit", () => process.exit(0))
   */
  onMenu(id: string, handler: () => void): () => void {
    return window.__vorn__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.id as string) === id) handler()
    })
  },
}
