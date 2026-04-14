use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InboundMsg {
    // Page / script
    InitScript { code: String },
    Navigate    { url: String },
    Eval        { code: String },

    // IPC return
    Return {
        id:     String,
        status: u8,
        result: serde_json::Value,
    },

    // Push event to frontend
    Event {
        name:    String,
        payload: serde_json::Value,
    },

    // Window lifecycle
    Terminate,
    Minimize,
    Maximize,
    Restore,
    Fullscreen { enter: bool },
    Center,
    Hide,
    Show,

    // Window properties
    SetTitle  { title: String },
    SetSize   { width: u32, height: u32 },
    SetMinSize { width: u32, height: u32 },
    SetMaxSize { width: u32, height: u32 },
    SetAlwaysOnTop { on: bool },
    SetPosition { x: i32, y: i32 },
    GetPosition { id: String },
    GetMonitors { id: String },

    // Window appearance
    SetFocus,
    SetEnabled    { enabled: bool },
    SetDecorations { decorated: bool },
    SetShadow      { shadow: bool },
    SetBackgroundColor { r: u8, g: u8, b: u8, a: u8 },
    SetTitleBarStyle   { style: String },
    SetVibrancy        { effect: String },
    SetButtons         { minimize: bool, maximize: bool, close: bool },
    PositionWindow     { position: String, monitor: Option<usize> },

    // Dialogs
    OpenFile      { id: String, options: OpenFileOpts },
    SaveFile      { id: String, options: SaveFileOpts },
    OpenDirectory { id: String, options: OpenDirOpts },
    MessageDialog {
        id: String,
        title: Option<String>,
        message: String,
        #[serde(rename = "dialogType")]
        dialog_type: String,
        #[serde(rename = "defaultValue")]
        default_value: Option<String>,
    },

    // Clipboard
    ClipboardRead  { id: String },
    ClipboardWrite { text: String },

    // Notifications
    Notify { title: String, body: String, icon: Option<String> },

    // System tray
    TrayCreate  { tooltip: Option<String>, icon: Option<String> },
    TraySetMenu { items: Vec<TrayItemDef> },
    TrayRemove,

    // Window menu
    MenuSet    { items: Vec<MenuItemDef> },
    MenuRemove,

    // Context menu
    ContextMenuShow {
        id:    String,
        items: Vec<MenuItemDef>,
        x:     Option<f64>,
        y:     Option<f64>,
    },

    // Global shortcuts
    ShortcutRegister {
        #[serde(rename = "shortcutId")]
        shortcut_id: String,
        accelerator: String,
    },
    ShortcutUnregister {
        #[serde(rename = "shortcutId")]
        shortcut_id: String,
    },

    // Window taskbar / attention
    SetSkipTaskbar { skip: bool },
    RequestUserAttention { critical: bool },

    // Theme
    GetTheme { id: String },

    // Content protection
    SetContentProtected { protected: bool },

    // Always on bottom
    SetAlwaysOnBottom { on: bool },

    // Clipboard extras
    ClipboardWriteHtml { html: String, text: Option<String> },
    ClipboardClear,

    // Progress bar
    SetProgressBar { progress: Option<f64> },

    // Hardware info
    GetSystemInfo    { id: String },
    GetCpuUsage      { id: String },
    GetMemoryInfo    { id: String },
    GetBatteryInfo   { id: String },
    GetDiskInfo      { id: String },
    GetNetworkInfo   { id: String },
    GetGpuUsage      { id: String },
    GetTemperature   { id: String },
    GetUsbDevices    { id: String },
    GetAiCapabilities { id: String },
    StartHwMonitor   { #[serde(rename = "intervalMs")] interval_ms: u64 },
    StopHwMonitor,
    GetNetworkSpeed  { id: String },
    GetProcessList   { id: String },
    GetUsers         { id: String },
    GetAudioDevices  { id: String },
    GetDisplayInfo   { id: String },
    GetCpuDetails    { id: String },
    GetRamDetails    { id: String },
    SetBadgeCount    { count: Option<i64> },

    // Internal: sent by IPC handler when the first real page is ready to show
    #[serde(skip)]
    ShowWindow,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenFileOpts {
    pub filters:      Option<Vec<FileFilter>>,
    pub multiple:     Option<bool>,
    pub default_path: Option<String>,
    pub title:        Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SaveFileOpts {
    pub filters:      Option<Vec<FileFilter>>,
    pub default_path: Option<String>,
    pub title:        Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OpenDirOpts {
    pub default_path: Option<String>,
    pub title:        Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FileFilter {
    pub name:       String,
    pub extensions: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct TrayItemDef {
    pub id:        String,
    pub label:     String,
    pub checked:   Option<bool>,
    pub enabled:   Option<bool>,
    pub separator: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct MenuItemDef {
    pub id:          Option<String>,
    pub label:       String,
    pub accelerator: Option<String>,
    pub checked:     Option<bool>,
    pub enabled:     Option<bool>,
    pub separator:   Option<bool>,
    pub submenu:     Option<Vec<MenuItemDef>>,
}
