use std::collections::HashMap;

use muda::{
    accelerator::Accelerator, CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem,
};
use tray_icon::TrayIconBuilder;

use crate::{
    proto::TrayItemDef,
    state::{AppState, MenuSource},
};

pub(super) fn tray_create(
    state:     &mut AppState,
    tooltip:   Option<String>,
    icon_path: Option<String>,
) {
    let icon = icon_path
        .as_deref()
        .and_then(load_icon)
        .or_else(default_icon);

    let Some(icon) = icon else {
        eprintln!("[bunview] trayCreate: could not load icon");
        return;
    };

    let mut builder = TrayIconBuilder::new().with_icon(icon);
    if let Some(ref tip) = tooltip {
        builder = builder.with_tooltip(tip);
    }

    match builder.build() {
        Ok(tray) => { state.tray = Some(tray); }
        Err(e)   => eprintln!("[bunview] trayCreate failed: {e}"),
    }
}

pub(super) fn tray_set_menu(state: &mut AppState, items: Vec<TrayItemDef>) {
    state.clear_menu_source(MenuSource::TrayMenu);

    let menu   = Menu::new();
    let mut id_map: HashMap<MenuId, (String, MenuSource)> = HashMap::new();

    for item in &items {
        if item.separator.unwrap_or(false) {
            let _ = menu.append(&PredefinedMenuItem::separator());
            continue;
        }
        let enabled = item.enabled.unwrap_or(true);
        if item.checked.unwrap_or(false) {
            let mi = CheckMenuItem::new(&item.label, enabled, false, None::<Accelerator>);
            id_map.insert(mi.id().clone(), (item.id.clone(), MenuSource::TrayMenu));
            let _ = menu.append(&mi);
        } else {
            let mi = MenuItem::new(&item.label, enabled, None::<Accelerator>);
            id_map.insert(mi.id().clone(), (item.id.clone(), MenuSource::TrayMenu));
            let _ = menu.append(&mi);
        }
    }

    state.menu_ids.extend(id_map);

    if let Some(tray) = &mut state.tray {
        tray.set_menu(Some(Box::new(menu)));
    }
}

fn load_icon(path: &str) -> Option<tray_icon::Icon> {
    let img    = image::open(path).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    tray_icon::Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn default_icon() -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(vec![0u8; 4], 1, 1).ok()
}
