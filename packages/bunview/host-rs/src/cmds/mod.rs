mod appearance;
mod dialog;
mod hardware;
mod menu;
mod platform;
mod shortcut;
mod tray;
mod window;

use arboard::Clipboard;
use muda::MenuEvent;
use serde_json::json;
use tao::window::{Fullscreen, Window};
use tray_icon::TrayIconEvent;
use wry::WebView;

use crate::{
    ipc,
    proto::InboundMsg,
    state::{AppState, MenuSource},
};

pub fn apply_vibrancy_initial(win: &Window, effect: &str) {
    appearance::set_vibrancy_impl(win, effect);
}

pub fn dispatch(msg: InboundMsg, win: &Window, webview: &WebView, state: &mut AppState) {
    match msg {
        InboundMsg::InitScript { .. } => {}
        InboundMsg::Navigate { url }  => { let _ = webview.load_url(&url); }
        InboundMsg::Eval { code }     => { let _ = webview.evaluate_script(&code); }
        InboundMsg::Return { id, status, result } => {
            let _ = webview.evaluate_script(&ipc::eval_resolve(&id, status, &result));
        }
        InboundMsg::Event { name, payload } => {
            let _ = webview.evaluate_script(&ipc::eval_dispatch(&name, &payload));
        }
        InboundMsg::Terminate => {
            win.set_visible(false);
            ipc::emit_close();
            std::process::exit(0);
        }
        InboundMsg::Minimize              => win.set_minimized(true),
        InboundMsg::Maximize              => win.set_maximized(true),
        InboundMsg::Restore               => { win.set_minimized(false); win.set_maximized(false); }
        InboundMsg::Fullscreen { enter }  => {
            if enter { win.set_fullscreen(Some(Fullscreen::Borderless(None))); }
            else     { win.set_fullscreen(None); }
        }
        InboundMsg::Center                => window::center_window(win),
        InboundMsg::Hide                  => win.set_visible(false),
        InboundMsg::Show                  => win.set_visible(true),
        InboundMsg::SetFocus              => win.set_focus(),
        InboundMsg::SetDecorations { decorated } => win.set_decorations(decorated),
        InboundMsg::SetAlwaysOnTop { on } => win.set_always_on_top(on),
        InboundMsg::SetTitle { title } => win.set_title(&title),
        InboundMsg::SetSize { width, height } => {
            win.set_inner_size(tao::dpi::LogicalSize::new(width, height));
        }
        InboundMsg::SetMinSize { width, height } => {
            win.set_min_inner_size(Some(tao::dpi::LogicalSize::new(width, height)));
        }
        InboundMsg::SetMaxSize { width, height } => {
            win.set_max_inner_size(Some(tao::dpi::LogicalSize::new(width, height)));
        }
        InboundMsg::SetPosition { x, y } => {
            win.set_outer_position(tao::dpi::LogicalPosition::new(x, y));
        }
        InboundMsg::GetPosition { id }   => window::get_position(win, &id),
        InboundMsg::GetMonitors { id }   => window::get_monitors(win, &id),
        InboundMsg::PositionWindow { position, monitor } => {
            window::position_window(win, &position, monitor, state);
        }
        InboundMsg::SetEnabled { enabled }              => appearance::set_enabled(win, enabled),
        InboundMsg::SetShadow { shadow }                => appearance::set_shadow(win, shadow),
        InboundMsg::SetBackgroundColor { r, g, b, a }  => {
            appearance::set_background_color(webview, r, g, b, a);
        }
        InboundMsg::SetTitleBarStyle { style }          => appearance::set_title_bar_style(win, &style),
        InboundMsg::SetVibrancy { effect }              => appearance::set_vibrancy_impl(win, &effect),
        InboundMsg::SetButtons { minimize, maximize, close } => {
            appearance::set_buttons(win, minimize, maximize, close);
        }
        InboundMsg::OpenFile { id, options }      => dialog::open_file(id, options),
        InboundMsg::SaveFile { id, options }      => dialog::save_file(id, options),
        InboundMsg::OpenDirectory { id, options } => dialog::open_directory(id, options),
        InboundMsg::MessageDialog { id, title, message, dialog_type, default_value } => {
            dialog::message_dialog(webview, id, title, message, dialog_type, default_value);
        }
        InboundMsg::ClipboardRead { id } => {
            let text = Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
            ipc::emit_response(&id, json!(text));
        }
        InboundMsg::ClipboardWrite { text } => {
            if let Ok(mut c) = Clipboard::new() { let _ = c.set_text(text); }
        }
        InboundMsg::ClipboardWriteHtml { html, text } => {
            if let Ok(mut c) = Clipboard::new() { let _ = c.set().html(html, text); }
        }
        InboundMsg::ClipboardClear => {
            if let Ok(mut c) = Clipboard::new() { let _ = c.clear(); }
        }
        InboundMsg::Notify { title, body, icon: _ } => {
            std::thread::spawn(move || {
                let _ = notify_rust::Notification::new().summary(&title).body(&body).show();
            });
        }
        InboundMsg::TrayCreate { tooltip, icon } => tray::tray_create(state, tooltip, icon),
        InboundMsg::TraySetMenu { items }         => tray::tray_set_menu(state, items),
        InboundMsg::TrayRemove => {
            state.tray = None;
            state.clear_menu_source(MenuSource::TrayMenu);
        }
        InboundMsg::MenuSet { items }  => menu::menu_set(win, state, items),
        InboundMsg::MenuRemove         => menu::menu_remove(win, state),
        InboundMsg::ContextMenuShow { id: _, items, x, y } => {
            menu::context_menu_show(win, state, items, x, y);
        }
        InboundMsg::ShortcutRegister { shortcut_id, accelerator } => {
            shortcut::shortcut_register(state, shortcut_id, accelerator);
        }
        InboundMsg::ShortcutUnregister { shortcut_id } => {
            shortcut::shortcut_unregister(state, &shortcut_id);
        }
        InboundMsg::SetSkipTaskbar { skip }           => platform::set_skip_taskbar(win, skip),
        InboundMsg::RequestUserAttention { critical } => platform::request_user_attention(win, critical),
        InboundMsg::SetContentProtected { protected } => platform::set_content_protected(win, protected),
        InboundMsg::SetAlwaysOnBottom { on } => {
            if on { win.set_always_on_top(false); }
            platform::set_always_on_bottom(win, on);
        }
        InboundMsg::SetProgressBar { progress } => platform::set_progress_bar(win, progress),
        InboundMsg::SetBadgeCount { count }     => platform::set_badge_count(win, count),
        InboundMsg::GetTheme { id } => {
            let theme = match win.theme() {
                tao::window::Theme::Dark => "dark",
                _ => "light",
            };
            ipc::emit_response(&id, json!(theme));
        }
        InboundMsg::GetSystemInfo     { id } => hardware::get_system_info(id),
        InboundMsg::GetCpuUsage       { id } => hardware::get_cpu_usage(id),
        InboundMsg::GetMemoryInfo     { id } => hardware::get_memory_info(id),
        InboundMsg::GetBatteryInfo    { id } => hardware::get_battery_info(id),
        InboundMsg::GetDiskInfo       { id } => hardware::get_disk_info(id),
        InboundMsg::GetNetworkInfo    { id } => hardware::get_network_info(id),
        InboundMsg::GetGpuUsage       { id } => hardware::get_gpu_usage(id),
        InboundMsg::GetTemperature    { id } => hardware::get_temperature(id),
        InboundMsg::GetUsbDevices     { id } => hardware::get_usb_devices(id),
        InboundMsg::GetAiCapabilities { id } => hardware::get_ai_capabilities(id),
        InboundMsg::StartHwMonitor { interval_ms } => hardware::start_hw_monitor(interval_ms),
        InboundMsg::StopHwMonitor => hardware::stop_hw_monitor(),
        InboundMsg::GetNetworkSpeed  { id } => hardware::get_network_speed(id),
        InboundMsg::GetProcessList   { id } => hardware::get_process_list(id),
        InboundMsg::GetUsers         { id } => hardware::get_users(id),
        InboundMsg::GetAudioDevices  { id } => hardware::get_audio_devices(id),
        InboundMsg::GetDisplayInfo   { id } => hardware::get_display_info(id),
        InboundMsg::GetCpuDetails    { id } => hardware::get_cpu_details(id),
        InboundMsg::GetRamDetails    { id } => hardware::get_ram_details(id),
        InboundMsg::ShowWindow => {
            win.set_visible(true);
            win.set_focus();
        }
    }
}

pub fn poll_events(state: &mut AppState) {
    // Tray icon clicks — TrayIconEvent is an enum since tray-icon 0.22
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if let TrayIconEvent::Click { position, .. } = event {
            state.last_tray_pos = Some((position.x, position.y));
            ipc::emit(json!({"type": "trayClick"}));
        }
    }

    // Menu item clicks (window menu, tray menu, context menu)
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        if let Some((user_id, source)) = state.menu_ids.get(&event.id) {
            let type_str = match source {
                MenuSource::TrayMenu    => "trayMenuItemClick",
                MenuSource::WindowMenu  => "menuItemClick",
                MenuSource::ContextMenu => "contextMenuItemClick",
            };
            ipc::emit(json!({"type": type_str, "id": user_id}));
        }
    }

    // Global hotkeys
    while let Ok(event) = global_hotkey::GlobalHotKeyEvent::receiver().try_recv() {
        if event.state == global_hotkey::HotKeyState::Pressed {
            if let Some(shortcut_id) = state.hotkey_ids.get(&event.id) {
                ipc::emit(json!({"type": "shortcutTriggered", "id": shortcut_id}));
            }
        }
    }
}
