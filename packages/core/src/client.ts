// @tynd/core/client — frontend API. Backend imports from "@tynd/core".
// Only the core surface lives here: app, backend RPC, the current window and
// webview, os, path, monitors, security, log, events and workers. Every other
// native API (dialog, clipboard, store, tray, menu, multiwindow, ...) ships as
// an official plugin: `import { dialog } from "@tynd/plugin-dialog/client"`.
// Plugin clients are **never** re-exported here. Plugin packages
// (`@tynd/plugin-*`) are separate npm dependencies — install only what
// you need, then import from the plugin's own package so the plugin
// stays the single source of truth for its types and client shape:
//     import { sql } from "@tynd/plugin-sql/client"
//     import { vault } from "@acme/plugin-vault/client"
// Swapping runtimes doesn't change where imports come from — preset
// and custom both resolve plugins from their own package.

export { base64ToBytes, bytesToBase64 } from "./client/_internal";
export {
  type ActivationPolicy,
  type AppInfo,
  app,
  type DockBounceType,
  type JumpList,
  type JumpListCategory,
  type JumpListItem,
} from "./client/app";
export { type BackendClient, type CallOptions, createBackend } from "./client/backend";
export { Channel } from "./client/channel";
export { createFrontendEmitter } from "./client/frontend-emitter";
export {
  type LogConfigureOptions,
  type LogLevel,
  type LogRotation,
  log,
} from "./client/log";
export {
  type Monitor,
  type MonitorChange,
  type MonitorRect,
  type MonitorSubscription,
  monitors,
} from "./client/monitor";
export {
  type MediaAccessKind,
  type MediaAccessStatus,
  type OsInfo,
  os,
} from "./client/os";
export { path } from "./client/path";
export { createPluginClient, type PluginClient } from "./client/plugin";
export { convertFileSrc, protocolUrl } from "./client/protocol";
export {
  abortable,
  listenN,
  once,
  RpcTimeoutError,
  withTimeout,
} from "./client/rpc-helpers";
export {
  type SecurityPolicy,
  type SecurityScope,
  security,
} from "./client/security";
export type { CommandCatalog, TypedClient } from "./client/typed-client";
export {
  tyndWebview,
  type WebviewCookie,
  type WebviewCookieInit,
  type WebviewCookieTarget,
  type WebviewDownloadCompleted,
  type WebviewDownloadEvent,
  type WebviewDownloadStarted,
  type WebviewNewWindowEvent,
  type WebviewPageSize,
  type WebviewPdfOptions,
  type WebviewTarget,
} from "./client/webview";
export {
  type AlwaysOnTopLevel,
  type AttentionKind,
  type DownloadPolicy,
  type NewWindowPolicy,
  type StartDragOptions,
  type TyndWindow,
  tyndWindow,
  type WindowOptions,
  type WindowPreset,
} from "./client/window";
export type {
  MacosEffect,
  WindowEffect,
  WindowEffectsOptions,
  WindowsEffect,
} from "./client/window-effects";
export { type WorkerHandle, workers } from "./client/workers";
export { TyndIpcError, type TyndIpcErrorCode } from "./errors";
export type { Emitter, EmitterMap } from "./types";
