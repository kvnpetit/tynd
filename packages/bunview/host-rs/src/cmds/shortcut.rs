use crate::state::AppState;

pub(super) fn shortcut_register(
    state:       &mut AppState,
    shortcut_id: String,
    accelerator: String,
) {
    use global_hotkey::{hotkey::HotKey, GlobalHotKeyManager};
    use std::str::FromStr;

    let manager = state
        .hotkey_manager
        .get_or_insert_with(|| GlobalHotKeyManager::new().expect("GlobalHotKeyManager::new"));

    match HotKey::from_str(&accelerator) {
        Ok(hotkey) => match manager.register(hotkey) {
            Ok(_)  => { state.hotkey_ids.insert(hotkey.id(), shortcut_id); }
            Err(e) => eprintln!("[bunview] shortcutRegister failed: {e}"),
        },
        Err(e) => eprintln!("[bunview] shortcutRegister parse '{accelerator}': {e:?}"),
    }
}

pub(super) fn shortcut_unregister(state: &mut AppState, shortcut_id: &str) {
    state.hotkey_ids.retain(|_, sid| sid.as_str() != shortcut_id);
}
