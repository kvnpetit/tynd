use crate::runtime::MenuItemDef;

pub(crate) fn build_bar(items: &[MenuItemDef]) -> Result<muda::Menu, String> {
    let menu = muda::Menu::new();
    for item in items {
        let label = item.label.as_deref().unwrap_or("");
        let enabled = item.enabled.unwrap_or(true);
        let sub = muda::Submenu::new(label, enabled);
        fill(
            &|i: &dyn muda::IsMenuItem| sub.append(i),
            item.items.as_deref().unwrap_or(&[]),
        )?;
        menu.append(&sub).map_err(|e| e.to_string())?;
    }
    Ok(menu)
}

/// Recursively fill a menu container using a caller-provided append closure.
pub(crate) fn fill(
    append: &dyn Fn(&dyn muda::IsMenuItem) -> muda::Result<()>,
    items: &[MenuItemDef],
) -> Result<(), String> {
    for item in items {
        match item.kind.as_deref() {
            Some("separator") => {
                append(&muda::PredefinedMenuItem::separator()).map_err(|e| e.to_string())?;
            },
            Some("submenu") => {
                let label = item.label.as_deref().unwrap_or("");
                let enabled = item.enabled.unwrap_or(true);
                let sub = muda::Submenu::new(label, enabled);
                fill(
                    &|i: &dyn muda::IsMenuItem| sub.append(i),
                    item.items.as_deref().unwrap_or(&[]),
                )?;
                append(&sub).map_err(|e| e.to_string())?;
            },
            _ => {
                if let Some(role) = &item.role {
                    if let Some(pi) = role_to_predefined(role) {
                        append(&pi).map_err(|e| e.to_string())?;
                        continue;
                    }
                }
                if let Some(label) = &item.label {
                    let id = muda::MenuId::new(item.id.as_deref().unwrap_or(label));
                    let mi = muda::MenuItem::with_id(id, label, item.enabled.unwrap_or(true), None);
                    append(&mi).map_err(|e| e.to_string())?;
                }
            },
        }
    }
    Ok(())
}

fn role_to_predefined(role: &str) -> Option<muda::PredefinedMenuItem> {
    use muda::PredefinedMenuItem as P;
    Some(match role {
        "separator" => P::separator(),
        "quit" => P::quit(None),
        "copy" => P::copy(None),
        "cut" => P::cut(None),
        "paste" => P::paste(None),
        "undo" => P::undo(None),
        "redo" => P::redo(None),
        "selectAll" => P::select_all(None),
        "minimize" => P::minimize(None),
        "close" => P::close_window(None),
        "about" => P::about(None::<&str>, None),
        _ => return None,
    })
}

pub(crate) fn init_bar(menu: &muda::Menu, window: &tao::window::Window) {
    #[cfg(target_os = "windows")]
    {
        use tao::platform::windows::WindowExtWindows;
        unsafe {
            if let Err(e) = menu.init_for_hwnd(window.hwnd()) {
                crate::vorn_log!("menu.init_for_hwnd: {e}");
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = window;
        menu.init_for_nsapp();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (menu, window);
    }
}
