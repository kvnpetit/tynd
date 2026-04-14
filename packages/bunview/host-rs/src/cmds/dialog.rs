use serde_json::json;
use wry::WebView;

use crate::{
    ipc,
    proto::{OpenDirOpts, OpenFileOpts, SaveFileOpts},
};

pub(super) fn open_file(id: String, options: OpenFileOpts) {
    std::thread::spawn(move || {
        let mut dlg = rfd::FileDialog::new();
        if let Some(t) = &options.title        { dlg = dlg.set_title(t); }
        if let Some(p) = &options.default_path { dlg = dlg.set_directory(p); }
        for f in options.filters.as_deref().unwrap_or(&[]) {
            let exts: Vec<&str> = f.extensions.iter().map(|s| s.as_str()).collect();
            dlg = dlg.add_filter(&f.name, &exts);
        }
        let result = if options.multiple.unwrap_or(false) {
            match dlg.pick_files() {
                Some(p) => json!(p.iter().map(|x| x.to_string_lossy().to_string()).collect::<Vec<_>>()),
                None    => serde_json::Value::Null,
            }
        } else {
            match dlg.pick_file() {
                Some(p) => json!(p.to_string_lossy().to_string()),
                None    => serde_json::Value::Null,
            }
        };
        ipc::emit_response(&id, result);
    });
}

pub(super) fn save_file(id: String, options: SaveFileOpts) {
    std::thread::spawn(move || {
        let mut dlg = rfd::FileDialog::new();
        if let Some(t) = &options.title        { dlg = dlg.set_title(t); }
        if let Some(p) = &options.default_path { dlg = dlg.set_directory(p); }
        for f in options.filters.as_deref().unwrap_or(&[]) {
            let exts: Vec<&str> = f.extensions.iter().map(|s| s.as_str()).collect();
            dlg = dlg.add_filter(&f.name, &exts);
        }
        let result = match dlg.save_file() {
            Some(p) => json!(p.to_string_lossy().to_string()),
            None    => serde_json::Value::Null,
        };
        ipc::emit_response(&id, result);
    });
}

pub(super) fn open_directory(id: String, options: OpenDirOpts) {
    std::thread::spawn(move || {
        let mut dlg = rfd::FileDialog::new();
        if let Some(t) = &options.title        { dlg = dlg.set_title(t); }
        if let Some(p) = &options.default_path { dlg = dlg.set_directory(p); }
        let result = match dlg.pick_folder() {
            Some(p) => json!(p.to_string_lossy().to_string()),
            None    => serde_json::Value::Null,
        };
        ipc::emit_response(&id, result);
    });
}

pub(super) fn message_dialog(
    webview:       &WebView,
    id:            String,
    title:         Option<String>,
    message:       String,
    dialog_type:   String,
    default_value: Option<String>,
) {
    if dialog_type.as_str() == "input" {
        // Inject an HTML overlay into the webview.
        // The result comes back via window.ipc.postMessage({__bv_dialog_result:true,...}).
        let prompt_text = message
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        let default_val = default_value.unwrap_or_default()
            .replace('\\', "\\\\")
            .replace('\'', "\\'");
        let dialog_id = id.replace('\'', "\\'");

        let script = format!(
            "(function(){{\
              var ov=document.createElement('div');\
              ov.id='__bv_dlg';\
              ov.style='position:fixed;inset:0;background:rgba(0,0,0,.45);z-index:2147483647;\
                        display:flex;align-items:center;justify-content:center;\
                        font-family:system-ui,-apple-system,sans-serif';\
              ov.innerHTML='<div style=\"background:#fff;border-radius:10px;padding:24px;\
                min-width:300px;max-width:480px;box-shadow:0 12px 40px rgba(0,0,0,.3)\">\
                <p style=\"margin:0 0 14px;font-size:14px;color:#1a1a1a\">{prompt_text}</p>\
                <input id=\"__bv_inp\" style=\"width:100%;box-sizing:border-box;padding:9px 12px;\
                  border:1.5px solid #d1d5db;border-radius:6px;font-size:14px;outline:none\" \
                  value=\"{default_val}\" />\
                <div style=\"display:flex;gap:8px;margin-top:14px;justify-content:flex-end\">\
                  <button id=\"__bv_cancel\" style=\"padding:7px 18px;border:1.5px solid #d1d5db;\
                    border-radius:6px;background:#fff;cursor:pointer;font-size:13px\">Cancel</button>\
                  <button id=\"__bv_ok\" style=\"padding:7px 18px;border:0;border-radius:6px;\
                    background:#0078d4;color:#fff;cursor:pointer;font-size:13px;font-weight:500\">OK</button>\
                </div></div>';\
              document.body.appendChild(ov);\
              var inp=document.getElementById('__bv_inp');inp.focus();inp.select();\
              function done(v){{ov.remove();\
                window.ipc.postMessage(JSON.stringify(\
                  {{__bv_dialog_result:true,__bv_dialog_id:'{dialog_id}',value:v}}\
                ));}}\
              document.getElementById('__bv_ok').onclick=function(){{done(inp.value);}};\
              document.getElementById('__bv_cancel').onclick=function(){{done(null);}};\
              inp.onkeydown=function(e){{\
                if(e.key==='Enter')done(inp.value);\
                if(e.key==='Escape')done(null);\
              }};\
            }})();"
        );
        let _ = webview.evaluate_script(&script);
    } else {
        std::thread::spawn(move || {
            let result = match dialog_type.as_str() {
                "alert" => {
                    rfd::MessageDialog::new()
                        .set_title(title.as_deref().unwrap_or(""))
                        .set_description(&message)
                        .set_level(rfd::MessageLevel::Info)
                        .set_buttons(rfd::MessageButtons::Ok)
                        .show();
                    json!(true)
                }
                "confirm" => {
                    let r = rfd::MessageDialog::new()
                        .set_title(title.as_deref().unwrap_or(""))
                        .set_description(&message)
                        .set_buttons(rfd::MessageButtons::OkCancel)
                        .show();
                    json!(r == rfd::MessageDialogResult::Ok)
                }
                _ => serde_json::Value::Null,
            };
            ipc::emit_response(&id, result);
        });
    }
}
