// Internal IPC protocol — JSON messages exchanged between the Bun backend
// and the Rust `webview-host` subprocess over stdin/stdout. Not part of the
// public API; consumers should never import these types directly.

import type { OpenFileOptions, SaveFileOptions, OpenDirectoryOptions } from "./dialogs";
import type { TrayMenuItem, MenuItem } from "./menus";
import type { HardwareMonitorData } from "./hardware";

// --- Backend → Host (commands the host must execute) ---

export interface HostCmd_InitScript   { type: "initScript";    code: string }
export interface HostCmd_Event        { type: "event";         name: string; payload: unknown }
export interface HostCmd_Navigate     { type: "navigate";      url: string }
export interface HostCmd_SetTitle     { type: "setTitle";      title: string }
export interface HostCmd_SetSize      { type: "setSize";       width: number; height: number }
export interface HostCmd_Return       { type: "return";        id: string; status: 0|1; result: unknown }
export interface HostCmd_Terminate    { type: "terminate" }
export interface HostCmd_Eval         { type: "eval";          code: string }
export interface HostCmd_Minimize     { type: "minimize" }
export interface HostCmd_Maximize     { type: "maximize" }
export interface HostCmd_Restore      { type: "restore" }
export interface HostCmd_Fullscreen   { type: "fullscreen";    enter: boolean }
export interface HostCmd_Center       { type: "center" }
export interface HostCmd_SetMinSize   { type: "setMinSize";    width: number; height: number }
export interface HostCmd_SetMaxSize   { type: "setMaxSize";    width: number; height: number }
export interface HostCmd_SetAlwaysOnTop { type: "setAlwaysOnTop"; on: boolean }
export interface HostCmd_Hide         { type: "hide" }
export interface HostCmd_Show         { type: "show" }
export interface HostCmd_OpenFile     { type: "openFile";     id: string; options: OpenFileOptions }
export interface HostCmd_SaveFile     { type: "saveFile";     id: string; options: SaveFileOptions }
export interface HostCmd_OpenDir      { type: "openDirectory"; id: string; options: OpenDirectoryOptions }
export interface HostCmd_ClipboardRead  { type: "clipboardRead"; id: string }
export interface HostCmd_ClipboardWrite { type: "clipboardWrite"; text: string }
export interface HostCmd_Notify       { type: "notify"; title: string; body: string; icon?: string }
export interface HostCmd_TrayCreate   { type: "trayCreate"; tooltip?: string; icon?: string }
export interface HostCmd_TraySetMenu  { type: "traySetMenu"; items: TrayMenuItem[] }
export interface HostCmd_TrayRemove   { type: "trayRemove" }
export interface HostCmd_MenuSet      { type: "menuSet"; items: MenuItem[] }
export interface HostCmd_MenuRemove   { type: "menuRemove" }
export interface HostCmd_GetPosition  { type: "getPosition"; id: string }
export interface HostCmd_SetPosition  { type: "setPosition"; x: number; y: number }
export interface HostCmd_ContextMenuShow { type: "contextMenuShow"; id: string; items: MenuItem[]; x?: number; y?: number }
export interface HostCmd_ShortcutRegister   { type: "shortcutRegister"; shortcutId: string; accelerator: string }
export interface HostCmd_ShortcutUnregister { type: "shortcutUnregister"; shortcutId: string }
export interface HostCmd_MessageDialog { type: "messageDialog"; id: string; title?: string; message: string; dialogType: string; defaultValue?: string }
export interface HostCmd_SetVibrancy      { type: "setVibrancy"; effect: string }
export interface HostCmd_SetButtons       { type: "setButtons"; minimize: boolean; maximize: boolean; close: boolean }
export interface HostCmd_PositionWindow   { type: "positionWindow"; position: string; monitor?: number }
export interface HostCmd_GetMonitors      { type: "getMonitors"; id: string }
export interface HostCmd_SetFocus         { type: "setFocus" }
export interface HostCmd_SetEnabled       { type: "setEnabled"; enabled: boolean }
export interface HostCmd_SetDecorations   { type: "setDecorations"; decorated: boolean }
export interface HostCmd_SetShadow        { type: "setShadow"; shadow: boolean }
export interface HostCmd_SetBgColor       { type: "setBackgroundColor"; r: number; g: number; b: number; a: number }
export interface HostCmd_SetTitleBarStyle { type: "setTitleBarStyle"; style: string }
export interface HostCmd_SetSkipTaskbar       { type: "setSkipTaskbar"; skip: boolean }
export interface HostCmd_RequestUserAttention { type: "requestUserAttention"; critical: boolean }
export interface HostCmd_GetTheme             { type: "getTheme"; id: string }
export interface HostCmd_SetContentProtected  { type: "setContentProtected"; protected: boolean }
export interface HostCmd_SetAlwaysOnBottom    { type: "setAlwaysOnBottom"; on: boolean }
export interface HostCmd_ClipboardWriteHtml   { type: "clipboardWriteHtml"; html: string; text?: string }
export interface HostCmd_ClipboardClear       { type: "clipboardClear" }
export interface HostCmd_SetProgressBar       { type: "setProgressBar"; progress: number | null }
export interface HostCmd_SetBadgeCount        { type: "setBadgeCount";  count: number | null }
export interface HostCmd_GetSystemInfo        { type: "getSystemInfo";    id: string }
export interface HostCmd_GetCpuUsage          { type: "getCpuUsage";       id: string }
export interface HostCmd_GetMemoryInfo        { type: "getMemoryInfo";     id: string }
export interface HostCmd_GetBatteryInfo       { type: "getBatteryInfo";    id: string }
export interface HostCmd_GetDiskInfo          { type: "getDiskInfo";       id: string }
export interface HostCmd_GetNetworkInfo       { type: "getNetworkInfo";    id: string }
export interface HostCmd_GetGpuUsage          { type: "getGpuUsage";       id: string }
export interface HostCmd_GetTemperature       { type: "getTemperature";    id: string }
export interface HostCmd_GetUsbDevices        { type: "getUsbDevices";     id: string }
export interface HostCmd_GetAiCapabilities    { type: "getAiCapabilities"; id: string }
export interface HostCmd_StartHwMonitor       { type: "startHwMonitor";    intervalMs: number }
export interface HostCmd_StopHwMonitor        { type: "stopHwMonitor" }
export interface HostCmd_GetNetworkSpeed      { type: "getNetworkSpeed";   id: string }
export interface HostCmd_GetProcessList       { type: "getProcessList";    id: string }
export interface HostCmd_GetUsers             { type: "getUsers";          id: string }
export interface HostCmd_GetAudioDevices      { type: "getAudioDevices";   id: string }
export interface HostCmd_GetDisplayInfo       { type: "getDisplayInfo";    id: string }
export interface HostCmd_GetCpuDetails        { type: "getCpuDetails";     id: string }
export interface HostCmd_GetRamDetails        { type: "getRamDetails";     id: string }

export type HostCmd =
  | HostCmd_InitScript
  | HostCmd_Event
  | HostCmd_Navigate
  | HostCmd_SetTitle
  | HostCmd_SetSize
  | HostCmd_Return
  | HostCmd_Terminate
  | HostCmd_Eval
  | HostCmd_Minimize
  | HostCmd_Maximize
  | HostCmd_Restore
  | HostCmd_Fullscreen
  | HostCmd_Center
  | HostCmd_SetMinSize
  | HostCmd_SetMaxSize
  | HostCmd_SetAlwaysOnTop
  | HostCmd_Hide
  | HostCmd_Show
  | HostCmd_OpenFile
  | HostCmd_SaveFile
  | HostCmd_OpenDir
  | HostCmd_ClipboardRead
  | HostCmd_ClipboardWrite
  | HostCmd_Notify
  | HostCmd_TrayCreate
  | HostCmd_TraySetMenu
  | HostCmd_TrayRemove
  | HostCmd_MenuSet
  | HostCmd_MenuRemove
  | HostCmd_GetPosition
  | HostCmd_SetPosition
  | HostCmd_ContextMenuShow
  | HostCmd_ShortcutRegister
  | HostCmd_ShortcutUnregister
  | HostCmd_MessageDialog
  | HostCmd_SetVibrancy
  | HostCmd_SetButtons
  | HostCmd_PositionWindow
  | HostCmd_GetMonitors
  | HostCmd_SetFocus
  | HostCmd_SetEnabled
  | HostCmd_SetDecorations
  | HostCmd_SetShadow
  | HostCmd_SetBgColor
  | HostCmd_SetTitleBarStyle
  | HostCmd_SetSkipTaskbar
  | HostCmd_RequestUserAttention
  | HostCmd_GetTheme
  | HostCmd_SetContentProtected
  | HostCmd_SetAlwaysOnBottom
  | HostCmd_ClipboardWriteHtml
  | HostCmd_ClipboardClear
  | HostCmd_SetProgressBar
  | HostCmd_SetBadgeCount
  | HostCmd_GetSystemInfo
  | HostCmd_GetCpuUsage
  | HostCmd_GetMemoryInfo
  | HostCmd_GetBatteryInfo
  | HostCmd_GetDiskInfo
  | HostCmd_GetNetworkInfo
  | HostCmd_GetGpuUsage
  | HostCmd_GetTemperature
  | HostCmd_GetUsbDevices
  | HostCmd_GetAiCapabilities
  | HostCmd_StartHwMonitor
  | HostCmd_StopHwMonitor
  | HostCmd_GetNetworkSpeed
  | HostCmd_GetProcessList
  | HostCmd_GetUsers
  | HostCmd_GetAudioDevices
  | HostCmd_GetDisplayInfo
  | HostCmd_GetCpuDetails
  | HostCmd_GetRamDetails;

// --- Host → Backend (events emitted by the host) ---

export interface HostEvt_Ready    { type: "ready" }
export interface HostEvt_Invoke   { type: "invoke"; id: string; command: string; payload: unknown }
export interface HostEvt_Close    { type: "close" }
export interface HostEvt_Response { type: "response"; id: string; result: unknown; error?: string }
export interface HostEvt_TrayClick     { type: "trayClick" }
export interface HostEvt_TrayMenuItem  { type: "trayMenuItemClick"; id: string }
export interface HostEvt_MenuItemClick { type: "menuItemClick"; id: string }
export interface HostEvt_ContextMenuClick { type: "contextMenuItemClick"; id: string }
export interface HostEvt_ShortcutTriggered { type: "shortcutTriggered"; id: string }
export interface HostEvt_WindowMoved       { type: "windowMoved"; x: number; y: number }
export interface HostEvt_WindowResized     { type: "windowResized"; width: number; height: number }
export interface HostEvt_WindowFocusChanged { type: "windowFocusChanged"; focused: boolean }
export interface HostEvt_FileDrop          { type: "fileDrop"; paths: string[] }
export interface HostEvt_FrontendEvent     { type: "frontendEvent"; name: string; payload: unknown }
export interface HostEvt_FileDragEnter     { type: "fileDragEnter"; paths: string[] }
export interface HostEvt_FileDragLeave     { type: "fileDragLeave" }
export interface HostEvt_ThemeChanged      { type: "themeChanged"; theme: "dark" | "light" }
export interface HostEvt_HwMonitorUpdate   { type: "hwMonitorUpdate"; cpu: HardwareMonitorData["cpu"]; memory: HardwareMonitorData["memory"]; temperatures: HardwareMonitorData["temperatures"]; timestamp: number }

export type HostEvt =
  | HostEvt_Ready
  | HostEvt_Invoke
  | HostEvt_Close
  | HostEvt_Response
  | HostEvt_TrayClick
  | HostEvt_TrayMenuItem
  | HostEvt_MenuItemClick
  | HostEvt_ContextMenuClick
  | HostEvt_ShortcutTriggered
  | HostEvt_WindowMoved
  | HostEvt_WindowResized
  | HostEvt_WindowFocusChanged
  | HostEvt_FileDrop
  | HostEvt_FrontendEvent
  | HostEvt_FileDragEnter
  | HostEvt_FileDragLeave
  | HostEvt_ThemeChanged
  | HostEvt_HwMonitorUpdate;
