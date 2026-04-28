// @tynd/core/client — frontend API. Backend imports from "@tynd/core".
//
// Plugin clients are **never** re-exported here. Official plugins
// (`@tynd/plugin-*`) are installed transitively when you depend on
// `@tynd/core` so the packages resolve without a second `bun add`,
// but you still import from the plugin's own package so the plugin
// stays the single source of truth for its types and client shape:
//
//     import { sql } from "@tynd/plugin-sql/client"
//     import { vault } from "@acme/plugin-vault/client"
//
// Swapping runtimes doesn't change where imports come from — preset
// and custom both resolve plugins from their own package.

export { base64ToBytes, bytesToBase64 } from "./client/_internal";
export { type AppInfo, app } from "./client/app";
export { type AutolaunchOptions, autolaunch } from "./client/autolaunch";
export { type BackendClient, createBackend } from "./client/backend";
export { type ClipboardImage, clipboard } from "./client/clipboard";
export { compute, type HashAlgo } from "./client/compute";
export {
  type ConfirmOptions,
  dialog,
  type MessageOptions,
  type OpenDirectoryOptions,
  type OpenFileOptions,
  type SaveFileOptions,
} from "./client/dialog";
export { createFrontendEmitter } from "./client/frontend-emitter";
export {
  type DirEntry,
  type FileHandle,
  type FileStat,
  type FsChangeEvent,
  fs,
  type OpenOptions,
  type SeekFrom,
  type WatchHandle,
} from "./client/fs";
export {
  type HttpProgress,
  type HttpRequestOptions,
  type HttpResponse,
  http,
} from "./client/http";
export { type KeyringEntry, keyring } from "./client/keyring";
export { type LogConfigureOptions, type LogLevel, log } from "./client/log";
export { type ContextMenuItem, menu } from "./client/menu";
export { type Monitor, monitors } from "./client/monitor";
export { type NotificationOptions, notification } from "./client/notification";
export { type OsInfo, os } from "./client/os";
export { path } from "./client/path";
export { createPluginClient, type PluginClient, type PluginShape } from "./client/plugin";
export { type PowerStatus, power } from "./client/power";
export { type ExecOptions, type ExecResult, process } from "./client/process";
export {
  abortable,
  listenN,
  once,
  RpcTimeoutError,
  withTimeout,
} from "./client/rpc-helpers";
export { type SecurityPolicy, type SecurityScope, security } from "./client/security";
export { shell } from "./client/shell";
export { type ShortcutHandle, shortcuts } from "./client/shortcuts";
export { sidecar } from "./client/sidecar";
export { type SingleInstanceResult, singleInstance } from "./client/single-instance";
export { createStore } from "./client/store";
export {
  type ConfigureOptions as ThemeConfigureOptions,
  type EffectiveTheme,
  type Palette,
  type ThemePreference,
  theme,
} from "./client/theme";
export { type TrayMenuItem, tray } from "./client/tray";
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
} from "./client/updater";

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
} from "./client/web";
export {
  type WebSocketHandle,
  type WebSocketMessage,
  websocket,
} from "./client/websocket";
export { tyndWindow, type WindowPreset } from "./client/window";
export { type WorkerHandle, workers } from "./client/workers";
export type { Emitter, EmitterMap } from "./types";
