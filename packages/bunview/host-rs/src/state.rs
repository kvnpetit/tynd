use std::collections::HashMap;
use global_hotkey::GlobalHotKeyManager;
use muda::MenuId;
use tray_icon::TrayIcon;

/// Which "source" a muda MenuId belongs to — used to emit the right event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuSource {
    WindowMenu,
    TrayMenu,
    ContextMenu,
}

pub struct AppState {
    // System tray handle (kept alive to show the icon)
    pub tray: Option<TrayIcon>,

    // muda MenuId → (user-string-id, source)
    pub menu_ids: HashMap<MenuId, (String, MenuSource)>,

    // global-hotkey id (u32) → user shortcut_id
    pub hotkey_ids: HashMap<u32, String>,

    // Hotkey manager (must stay alive for hotkeys to fire)
    pub hotkey_manager: Option<GlobalHotKeyManager>,

    // Current window menu (kept alive)
    pub window_menu: Option<muda::Menu>,

    // Current context menu (rebuilt each show)
    pub context_menu: Option<muda::Menu>,

    // Last tray icon click cursor position (for trayLeft/trayRight positioning)
    pub last_tray_pos: Option<(f64, f64)>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tray:           None,
            menu_ids:       HashMap::new(),
            hotkey_ids:     HashMap::new(),
            hotkey_manager: None,
            window_menu:    None,
            context_menu:   None,
            last_tray_pos:  None,
        }
    }

    /// Remove all menu IDs belonging to the given source.
    pub fn clear_menu_source(&mut self, source: MenuSource) {
        self.menu_ids.retain(|_, (_, s)| *s != source);
    }
}
