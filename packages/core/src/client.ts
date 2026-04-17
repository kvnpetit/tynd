// @tynd/core/client — frontend API. Backend imports from "@tynd/core".

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

import { tynd } from "./logger.js"

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

declare global {
  interface Window {
    __tynd__: {
      call(fn: string, args: unknown[]): Promise<unknown>
      os_call(api: string, method: string, args: unknown): Promise<unknown>
      os_on(name: string, handler: (data: unknown) => void): () => void
      on(name: string, handler: (payload: unknown) => void): () => void
      off(name: string, handler: (payload: unknown) => void): void
    }
    __tynd_os_result__: (id: string, ok: boolean, value: unknown) => void
    __tynd_os_event__: (name: string, data: unknown) => void
  }
}

/**
 * Create a fully type-safe proxy to the backend.
 *
 * @example
 * import type * as backend from "../backend/main"
 * const api = createBackend<typeof backend>()
 * await api.greet("Alice")
 */
export function createBackend<T>(): BackendClient<T> {
  return new Proxy({} as BackendClient<T>, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return undefined
      // A thenable Proxy would make `await api` invoke `api.then()` as a backend call.
      if (prop === "then" || prop === "catch" || prop === "finally") return undefined

      if (!window.__tynd__) {
        tynd.error("window.__tynd__ is not available — are you running outside a tynd app?")
        return () => Promise.reject(new Error("[tynd] not in a tynd app context"))
      }

      // Event subscription helpers
      if (prop === "on") {
        return (event: string, handler: (p: unknown) => void) => window.__tynd__.on(event, handler)
      }

      if (prop === "once") {
        return (event: string, handler: (p: unknown) => void) => {
          let called = false
          const wrapper = (p: unknown) => {
            if (!called) {
              called = true
              window.__tynd__.off(event, wrapper)
              handler(p)
            }
          }
          window.__tynd__.on(event, wrapper)
          return () => window.__tynd__.off(event, wrapper)
        }
      }

      // Default: proxy as a backend function call
      return (...args: unknown[]) => window.__tynd__.call(prop, args)
    },
  })
}

function _osCall<T>(api: string, method: string, args: unknown = null): Promise<T> {
  return window.__tynd__.os_call(api, method, args) as Promise<T>
}

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

/**
 * Control the native application window from the frontend.
 *
 * @example
 * await tyndWindow.setTitle("My App — Unsaved")
 * await tyndWindow.maximize()
 */
export const tyndWindow = {
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
   * tyndWindow.onMenu("file.open", () => openFile())
   */
  onMenu(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
}

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

/**
 * Subscribe to system tray events.
 * The tray itself is configured in `app.start({ tray: { ... } })`.
 *
 * @example
 * tray.onClick(() => tyndWindow.show())
 * tray.onMenu("show", () => tyndWindow.show())
 */
export const tray = {
  /** Fires when the tray icon is left-clicked. */
  onClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:click", () => handler())
  },

  /** Fires when the tray icon is right-clicked. */
  onRightClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:right-click", () => handler())
  },

  /** Fires when the tray icon is double-clicked. */
  onDoubleClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:double-click", () => handler())
  },

  /**
   * Subscribe to a tray context menu item click by its `id`.
   * Returns an unsubscribe function.
   *
   * @example
   * tray.onMenu("quit", () => process.exit(0))
   */
  onMenu(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
}

export interface ExecOptions {
  args?: string[]
  cwd?: string
  env?: Record<string, string>
  input?: string
}

export interface ExecResult {
  code: number | null
  stdout: string
  stderr: string
}

export const process = {
  exec(cmd: string, opts?: ExecOptions): Promise<ExecResult> {
    return _osCall("process", "exec", { cmd, ...opts })
  },
  execShell(cmd: string, opts?: Omit<ExecOptions, "args">): Promise<ExecResult> {
    return _osCall("process", "execShell", { cmd, ...opts })
  },
}

export interface FileStat {
  size: number
  isFile: boolean
  isDir: boolean
  isSymlink: boolean
  mtime: number | null
}

export interface DirEntry {
  name: string
  isFile: boolean
  isDir: boolean
  isSymlink: boolean
}

export const fs = {
  readText(path: string): Promise<string> {
    return _osCall("fs", "readText", { path })
  },
  writeText(path: string, content: string, opts?: { createDirs?: boolean }): Promise<void> {
    return _osCall("fs", "writeText", { path, content, ...opts })
  },
  exists(path: string): Promise<boolean> {
    return _osCall("fs", "exists", { path })
  },
  stat(path: string): Promise<FileStat> {
    return _osCall("fs", "stat", { path })
  },
  readDir(path: string): Promise<DirEntry[]> {
    return _osCall("fs", "readDir", { path })
  },
  mkdir(path: string, opts?: { recursive?: boolean }): Promise<void> {
    return _osCall("fs", "mkdir", { path, ...opts })
  },
  remove(path: string, opts?: { recursive?: boolean }): Promise<void> {
    return _osCall("fs", "remove", { path, ...opts })
  },
  rename(from: string, to: string): Promise<void> {
    return _osCall("fs", "rename", { from, to })
  },
  copy(from: string, to: string): Promise<void> {
    return _osCall("fs", "copy", { from, to })
  },
}

export function createStore(namespace: string) {
  return {
    get<T = unknown>(key: string): Promise<T | null> {
      return _osCall("store", "get", { namespace, key }) as Promise<T | null>
    },
    set(key: string, value: unknown): Promise<void> {
      return _osCall("store", "set", { namespace, key, value })
    },
    delete(key: string): Promise<void> {
      return _osCall("store", "delete", { namespace, key })
    },
    clear(): Promise<void> {
      return _osCall("store", "clear", { namespace })
    },
    keys(): Promise<string[]> {
      return _osCall("store", "keys", { namespace })
    },
  }
}

export interface OsInfo {
  platform: "linux" | "macos" | "windows" | string
  arch: string
  family: string
}

export const os = {
  info(): Promise<OsInfo> {
    return _osCall("os", "info")
  },
  homeDir(): Promise<string | null> {
    return _osCall("os", "homeDir")
  },
  tmpDir(): Promise<string> {
    return _osCall("os", "tmpDir") as Promise<string>
  },
  configDir(): Promise<string | null> {
    return _osCall("os", "configDir")
  },
  dataDir(): Promise<string | null> {
    return _osCall("os", "dataDir")
  },
  cacheDir(): Promise<string | null> {
    return _osCall("os", "cacheDir")
  },
  exePath(): Promise<string | null> {
    return _osCall("os", "exePath")
  },
  cwd(): Promise<string | null> {
    return _osCall("os", "cwd")
  },
  env(key: string): Promise<string | null> {
    return _osCall("os", "env", { key })
  },
}

export const path = {
  sep(): "/" | "\\" {
    return typeof navigator !== "undefined" && /win/i.test(navigator.platform) ? "\\" : "/"
  },
  join(...parts: string[]): string {
    const s = path.sep()
    return parts
      .filter((p) => p.length > 0)
      .join(s)
      .replace(/[/\\]+/g, s)
  },
  dirname(p: string): string {
    const m = p.replace(/[/\\]$/, "").match(/^(.*)[/\\][^/\\]+$/)
    return m ? m[1]! : ""
  },
  basename(p: string, ext?: string): string {
    const name =
      p
        .replace(/[/\\]$/, "")
        .split(/[/\\]/)
        .pop() ?? ""
    return ext && name.endsWith(ext) ? name.slice(0, -ext.length) : name
  },
  extname(p: string): string {
    const name = path.basename(p)
    const i = name.lastIndexOf(".")
    return i > 0 ? name.slice(i) : ""
  },
}
