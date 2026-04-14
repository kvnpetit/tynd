use std::collections::HashMap;

use muda::{
    accelerator::Accelerator, CheckMenuItem, ContextMenu as MudaContextMenu,
    IsMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu,
};
use tao::window::Window;

use crate::{
    proto::MenuItemDef,
    state::{AppState, MenuSource},
};

pub(super) fn menu_set(window: &Window, state: &mut AppState, items: Vec<MenuItemDef>) {
    state.clear_menu_source(MenuSource::WindowMenu);
    if let Some(old) = &state.window_menu {
        remove_for_window(window, old);
    }

    let menu   = Menu::new();
    let mut id_map: HashMap<MenuId, (String, MenuSource)> = HashMap::new();
    build_items(&|i| { let _ = menu.append(i); }, &items, &mut id_map, MenuSource::WindowMenu);
    state.menu_ids.extend(id_map);
    attach_to_window(window, &menu);
    state.window_menu = Some(menu);
}

pub(super) fn menu_remove(window: &Window, state: &mut AppState) {
    if let Some(menu) = &state.window_menu {
        remove_for_window(window, menu);
    }
    state.window_menu = None;
    state.clear_menu_source(MenuSource::WindowMenu);
}

pub(super) fn context_menu_show(
    window: &Window,
    state:  &mut AppState,
    items:  Vec<MenuItemDef>,
    _x:     Option<f64>,
    _y:     Option<f64>,
) {
    state.clear_menu_source(MenuSource::ContextMenu);

    let menu   = Menu::new();
    let mut id_map: HashMap<MenuId, (String, MenuSource)> = HashMap::new();
    build_items(&|i| { let _ = menu.append(i); }, &items, &mut id_map, MenuSource::ContextMenu);
    state.menu_ids.extend(id_map);

    // Show at current cursor position (None = use OS cursor)
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { menu.show_context_menu_for_hwnd(window.hwnd() as _, None::<muda::dpi::Position>); }
    }
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::WindowExtMacOS;
        unsafe {
            menu.show_context_menu_for_nsview(
                window.ns_view() as _,
                None::<muda::dpi::Position>,
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        menu.show_context_menu_for_gtk_window(
            window.gtk_window().as_ref(),
            None::<muda::dpi::Position>,
        );
    }

    state.context_menu = Some(menu);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn attach_to_window(window: &Window, menu: &Menu) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { let _ = menu.init_for_hwnd(window.hwnd() as _); }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        menu.init_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu.init_for_gtk_window(window.gtk_window().as_ref(), None);
    }
}

fn remove_for_window(window: &Window, menu: &Menu) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe { let _ = menu.remove_for_hwnd(window.hwnd() as _); }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        menu.remove_for_nsapp();
    }
    #[cfg(target_os = "linux")]
    {
        use tao::platform::unix::WindowExtUnix;
        let _ = menu.remove_for_gtk_window(window.gtk_window().as_ref());
    }
}

/// Build menu items recursively.
/// `append` is a closure that appends an item to the parent container —
/// works for both `Menu` and `Submenu` without needing a shared trait.
fn build_items(
    append: &dyn Fn(&dyn IsMenuItem),
    items:  &[MenuItemDef],
    id_map: &mut HashMap<MenuId, (String, MenuSource)>,
    source: MenuSource,
) {
    for item in items {
        if item.separator.unwrap_or(false) {
            append(&PredefinedMenuItem::separator());
            continue;
        }
        let enabled = item.enabled.unwrap_or(true);
        let accel   = item.accelerator.as_deref().and_then(parse_accel);

        if let Some(ref sub_items) = item.submenu {
            let sub = Submenu::new(&item.label, enabled);
            build_items(&|i| { let _ = sub.append(i); }, sub_items, id_map, source);
            append(&sub);
        } else if item.checked.unwrap_or(false) {
            let mi = CheckMenuItem::new(&item.label, enabled, false, accel);
            if let Some(ref uid) = item.id {
                id_map.insert(mi.id().clone(), (uid.clone(), source));
            }
            append(&mi);
        } else {
            let mi = MenuItem::new(&item.label, enabled, accel);
            if let Some(ref uid) = item.id {
                id_map.insert(mi.id().clone(), (uid.clone(), source));
            }
            append(&mi);
        }
    }
}

fn parse_accel(s: &str) -> Option<Accelerator> {
    use std::str::FromStr;
    Accelerator::from_str(s).ok()
}
