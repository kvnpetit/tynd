use serde_json::Value;

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

/// Runtime tray mutations — must run on the main thread. Called via the
/// `UserEvent::TrayCmd` path so Windows / macOS invariants are preserved.
pub(crate) fn dispatch(
    tray: &tray_icon::TrayIcon,
    method: &str,
    args: &Value,
) -> Result<Value, String> {
    match method {
        "setIcon" => {
            let path = args
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "tray.setIcon: missing 'path'".to_string())?;
            let icon = os::icon::load_tray(path)?;
            tray.set_icon(Some(icon)).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "setTooltip" => {
            let text = args.get("text").and_then(Value::as_str);
            tray.set_tooltip(text).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "setTitle" => {
            // macOS only — sets text next to the icon in the menu bar.
            let text = args.get("text").and_then(Value::as_str);
            tray.set_title(text);
            Ok(Value::Null)
        },
        "setVisible" => {
            let visible = args.get("visible").and_then(Value::as_bool).unwrap_or(true);
            tray.set_visible(visible).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        },
        "setMenu" => {
            let items = args.get("items").and_then(Value::as_array).cloned();
            let new_menu = muda::Menu::new();
            if let Some(items) = items {
                // Re-use the TS menu schema — items carry the same shape as
                // tynd.config.ts::tray.menu, so we delegate to the shared
                // builder instead of duplicating the walk here.
                let parsed: Vec<crate::runtime::MenuItemDef> =
                    serde_json::from_value(Value::Array(items))
                        .map_err(|e| format!("tray.setMenu: invalid items — {e}"))?;
                menu::fill(&|i: &dyn muda::IsMenuItem| new_menu.append(i), &parsed)?;
            }
            tray.set_menu(Some(Box::new(new_menu)));
            Ok(Value::Null)
        },
        "setIconAsTemplate" => {
            // macOS: template icons automatically adapt to light / dark menu
            // bar without requiring a separate image. No-op on Win / Linux.
            let is_template = args
                .get("template")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            tray.set_icon_as_template(is_template);
            Ok(Value::Null)
        },
        _ => Err(format!("tray.{method}: unknown method")),
    }
}
