// @tynd/core/client — frontend API. Backend imports from "@tynd/core".

export { type BackendClient, createBackend } from "./client/backend.js"
export { clipboard } from "./client/clipboard.js"
export { compute, type HashAlgo } from "./client/compute.js"
export {
  type ConfirmOptions,
  dialog,
  type MessageOptions,
  type OpenFileOptions,
  type SaveFileOptions,
} from "./client/dialog.js"
export { type DirEntry, type FileStat, fs } from "./client/fs.js"
export {
  type HttpProgress,
  type HttpRequestOptions,
  type HttpResponse,
  http,
} from "./client/http.js"
export { menu } from "./client/menu.js"
export { type NotificationOptions, notification } from "./client/notification.js"
export { type OsInfo, os } from "./client/os.js"
export { path } from "./client/path.js"
export { type ExecOptions, type ExecResult, process } from "./client/process.js"
export { shell } from "./client/shell.js"
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
export { tray } from "./client/tray.js"
export {
  type UpdateInfo,
  type UpdaterCheckOptions,
  type UpdaterDownloadOptions,
  type UpdaterDownloadResult,
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
export { tyndWindow } from "./client/window.js"
export { type WorkerHandle, workers } from "./client/workers.js"
export type { Emitter, EmitterMap } from "./types.js"
