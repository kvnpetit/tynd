// @tynd/core/client — frontend API. Backend imports from "@tynd/core".

export { type AppInfo, app } from "./client/app.js"
export { type AutolaunchOptions, autolaunch } from "./client/autolaunch.js"
export { type BackendClient, createBackend } from "./client/backend.js"
export { createFrontendEmitter } from "./client/frontend-emitter.js"
export {
  abortable,
  listenN,
  once,
  RpcTimeoutError,
  withTimeout,
} from "./client/rpc-helpers.js"
export { type ClipboardImage, clipboard } from "./client/clipboard.js"
export { compute, type HashAlgo } from "./client/compute.js"
export {
  type ConfirmOptions,
  dialog,
  type MessageOptions,
  type OpenDirectoryOptions,
  type OpenFileOptions,
  type SaveFileOptions,
} from "./client/dialog.js"
export {
  type DirEntry,
  type FileHandle,
  type FileStat,
  type FsChangeEvent,
  fs,
  type OpenOptions,
  type SeekFrom,
  type WatchHandle,
} from "./client/fs.js"
export {
  type HttpProgress,
  type HttpRequestOptions,
  type HttpResponse,
  http,
} from "./client/http.js"
export { type KeyringEntry, keyring } from "./client/keyring.js"
export { log, type LogConfigureOptions, type LogLevel } from "./client/log.js"
export { type ContextMenuItem, menu } from "./client/menu.js"
export { type Monitor, monitors } from "./client/monitor.js"
export { type NotificationOptions, notification } from "./client/notification.js"
export { type OsInfo, os } from "./client/os.js"
export { path } from "./client/path.js"
export { power } from "./client/power.js"
export { security, type SecurityPolicy, type SecurityScope } from "./client/security.js"
export { type ExecOptions, type ExecResult, process } from "./client/process.js"
export { shell } from "./client/shell.js"
export { type ShortcutHandle, shortcuts } from "./client/shortcuts.js"
export { sidecar } from "./client/sidecar.js"
export { type SingleInstanceResult, singleInstance } from "./client/single-instance.js"
export {
  type SqlConnection,
  type SqlExecResult,
  type SqlParam,
  sql,
} from "./client/sql.js"
export { createStore } from "./client/store.js"
export {
  type TerminalHandle,
  type TerminalSpawnOptions,
  terminal,
} from "./client/terminal.js"
export { tray, type TrayMenuItem } from "./client/tray.js"
export {
  type UpdateInfo,
  type UpdaterCheckEvent,
  type UpdaterCheckOptions,
  type UpdaterDownloadOptions,
  type UpdaterDownloadResult,
  type UpdaterInstallOptions,
  type UpdaterInstallResult,
  type UpdaterPeriodicOptions,
  type UpdaterProgress,
  updater,
} from "./client/updater.js"
// Web-platform globals re-exported so `import * as tynd from "@tynd/core/client"`
// gives you `tynd.fetch`, `tynd.WebSocket`, `tynd.crypto`, etc. without
// touching the globals.
export {
  AbortController,
  AbortSignal,
  atob,
  Blob,
  btoa,
  crypto,
  EventSource,
  File,
  FormData,
  fetch,
  Headers,
  performance,
  ReadableStream,
  Request,
  Response,
  structuredClone,
  TextDecoder,
  TextEncoder,
  URL,
  URLSearchParams,
  WebSocket,
} from "./client/web.js"
export {
  type WebSocketHandle,
  type WebSocketMessage,
  websocket,
} from "./client/websocket.js"
export { tyndWindow, type WindowPreset } from "./client/window.js"
export { type WorkerHandle, workers } from "./client/workers.js"
export type { Emitter, EmitterMap } from "./types.js"
