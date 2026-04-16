use crate::{menu, os, runtime::TrayConfig};

pub(crate) fn build(cfg: &TrayConfig) -> Result<tray_icon::TrayIcon, String> {
    let icon = os::icon::load_tray(&cfg.icon)?;

    let tray_menu = muda::Menu::new();
    if let Some(items) = &cfg.menu {
        menu::fill(&|i: &dyn muda::IsMenuItem| tray_menu.append(i), items)?;
    }

    let mut builder = tray_icon::TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(tray_menu));

    if let Some(tt) = &cfg.tooltip {
        builder = builder.with_tooltip(tt);
    }

    builder.build().map_err(|e| e.to_string())
}
