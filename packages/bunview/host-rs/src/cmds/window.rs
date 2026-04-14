use serde_json::json;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    window::Window,
};

use crate::{ipc, state::AppState};

pub(super) fn center_window(window: &Window) {
    let Some(monitor) = window.current_monitor() else { return };
    let mon_pos  = monitor.position();
    let mon_size = monitor.size();
    let win_size = window.outer_size();
    let x = mon_pos.x + (mon_size.width  as i32 - win_size.width  as i32) / 2;
    let y = mon_pos.y + (mon_size.height as i32 - win_size.height as i32) / 2;
    window.set_outer_position(PhysicalPosition::new(x, y));
}

pub(super) fn get_position(window: &Window, id: &str) {
    let outer_pos  = window.outer_position().unwrap_or_default();
    let outer_size = window.outer_size();
    let logical: LogicalSize<f64> = outer_size.to_logical(window.scale_factor());
    ipc::emit_response(id, json!({
        "x": outer_pos.x, "y": outer_pos.y,
        "width":  logical.width  as u32,
        "height": logical.height as u32,
    }));
}

pub(super) fn get_monitors(window: &Window, id: &str) {
    let monitors: Vec<serde_json::Value> = window
        .available_monitors()
        .enumerate()
        .map(|(i, m)| {
            let pos     = m.position();
            let size    = m.size();
            let primary = window
                .primary_monitor()
                .map(|pm| pm.name() == m.name())
                .unwrap_or(i == 0);
            json!({
                "x": pos.x,  "y": pos.y,
                "width": size.width, "height": size.height,
                "primary": primary,
            })
        })
        .collect();
    ipc::emit_response(id, json!(monitors));
}

pub(super) fn position_window(
    window:      &Window,
    position:    &str,
    _monitor_idx: Option<usize>,
    state:       &AppState,
) {
    // Tray-relative positions use the last known cursor position
    if position == "trayLeft" || position == "trayRight" {
        if let Some((tx, ty)) = state.last_tray_pos {
            let win_size = window.outer_size();
            let ww = win_size.width  as i32;
            let wh = win_size.height as i32;
            let (x, y) = if position == "trayLeft" {
                (tx as i32 - ww, ty as i32 - wh)
            } else {
                (tx as i32, ty as i32 - wh)
            };
            window.set_outer_position(PhysicalPosition::new(x, y));
        }
        return;
    }

    let Some(monitor) = window.current_monitor() else { return };
    let mon_pos  = monitor.position();
    let mon_size = monitor.size();
    let win_size = window.outer_size();
    let mw = mon_size.width  as i32;
    let mh = mon_size.height as i32;
    let ww = win_size.width  as i32;
    let wh = win_size.height as i32;

    let (x, y): (i32, i32) = match position {
        "topLeft"      => (0, 0),
        "topCenter"    => ((mw - ww) / 2, 0),
        "topRight"     => (mw - ww, 0),
        "centerLeft"   => (0, (mh - wh) / 2),
        "center"       => ((mw - ww) / 2, (mh - wh) / 2),
        "centerRight"  => (mw - ww, (mh - wh) / 2),
        "bottomLeft"   => (0, mh - wh),
        "bottomCenter" => ((mw - ww) / 2, mh - wh),
        "bottomRight"  => (mw - ww, mh - wh),
        _              => return,
    };
    window.set_outer_position(PhysicalPosition::new(mon_pos.x + x, mon_pos.y + y));
}
