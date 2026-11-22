//! `startDrag({ files, icon? })`: drag files out of the window into another
//! app (Explorer, Finder, a mail client…). Runs on the main thread with the
//! real window, from a renderer `mousedown` / `dragstart` handler while the
//! button is still held.

use std::path::PathBuf;

use serde_json::Value;
use tao::window::Window;

const MAX_FILES: usize = 1024;
const MAX_ICON_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq)]
enum DragIcon {
    Default,
    File(PathBuf),
    Bytes(Vec<u8>),
}

#[derive(Debug)]
struct DragRequest {
    files: Vec<PathBuf>,
    icon: DragIcon,
}

/// Validate the renderer arguments. Every path (files and icon) goes through
/// the filesystem policy and capability roots; the authorized, canonical path
/// is what gets dragged.
fn parse_drag_request(
    args: &Value,
    authorize: impl Fn(&str) -> Result<PathBuf, String>,
) -> Result<DragRequest, String> {
    let files = args
        .get("files")
        .and_then(Value::as_array)
        .ok_or("startDrag: 'files' must be an array of absolute paths")?;
    if files.is_empty() || files.len() > MAX_FILES {
        return Err(format!(
            "startDrag: 'files' must hold 1 to {MAX_FILES} paths"
        ));
    }
    let files = files
        .iter()
        .map(|file| {
            let path = file
                .as_str()
                .ok_or("startDrag: every entry of 'files' must be a string")?;
            if !std::path::Path::new(path).is_absolute() {
                return Err(format!("startDrag: '{path}' is not an absolute path"));
            }
            authorize(path).map_err(|error| format!("startDrag: {error}"))
        })
        .collect::<Result<Vec<_>, String>>()?;

    let icon = match (args.get("iconPath"), args.get("iconBytes")) {
        (Some(Value::String(path)), None) => {
            DragIcon::File(authorize(path).map_err(|error| format!("startDrag: icon: {error}"))?)
        },
        (None, Some(Value::Array(bytes))) => {
            if bytes.len() > MAX_ICON_BYTES {
                return Err(format!(
                    "startDrag: icon is larger than {MAX_ICON_BYTES} bytes"
                ));
            }
            DragIcon::Bytes(
                bytes
                    .iter()
                    .map(|byte| byte.as_u64().and_then(|b| u8::try_from(b).ok()))
                    .collect::<Option<Vec<u8>>>()
                    .ok_or("startDrag: icon bytes must be 0-255")?,
            )
        },
        (None | Some(Value::Null), None | Some(Value::Null)) => DragIcon::Default,
        _ => return Err("startDrag: 'icon' must be an image path or PNG bytes".into()),
    };
    Ok(DragRequest { files, icon })
}

pub(super) fn start_drag(win: &Window, args: &Value) -> Result<Value, String> {
    let request = parse_drag_request(args, crate::security::policy::authorize_fs_path)?;
    let image = match request.icon {
        DragIcon::File(path) => drag::Image::File(path),
        DragIcon::Bytes(bytes) => drag::Image::Raw(bytes),
        DragIcon::Default => drag::Image::Raw(default_icon()),
    };
    // The GTK backend wants the `gtk::ApplicationWindow`, not the tao window.
    #[cfg(target_os = "linux")]
    let win = {
        use tao::platform::unix::WindowExtUnix;
        win.gtk_window()
    };
    drag::start_drag(
        win,
        drag::DragItem::Files(request.files),
        image,
        |_result, _cursor| {},
        drag::Options::default(),
    )
    .map_err(|error| format!("startDrag: {error}"))?;
    Ok(Value::Null)
}

/// Windows and Linux fall back to the shell's file icon when the image is
/// empty; AppKit needs a real image, so macOS gets a neutral 32 px tile.
#[cfg(target_os = "macos")]
fn default_icon() -> Vec<u8> {
    let tile = image::RgbaImage::from_pixel(32, 32, image::Rgba([128, 128, 128, 160]));
    let mut png = std::io::Cursor::new(Vec::new());
    if tile.write_to(&mut png, image::ImageFormat::Png).is_err() {
        return Vec::new();
    }
    png.into_inner()
}

#[cfg(not(target_os = "macos"))]
fn default_icon() -> Vec<u8> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn allow(path: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(path))
    }

    fn absolute(name: &str) -> String {
        std::env::temp_dir()
            .join(name)
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn files_must_be_a_non_empty_list_of_absolute_paths() {
        for args in [json!({}), json!({ "files": [] }), json!({ "files": [1] })] {
            assert!(parse_drag_request(&args, allow).is_err(), "{args}");
        }
        let error = parse_drag_request(&json!({ "files": ["relative.txt"] }), allow).unwrap_err();
        assert!(error.contains("absolute"), "{error}");

        let file = absolute("report.pdf");
        let request = parse_drag_request(&json!({ "files": [file] }), allow).unwrap();
        assert_eq!(request.files, vec![PathBuf::from(&file)]);
        assert_eq!(request.icon, DragIcon::Default);
    }

    #[test]
    fn every_path_goes_through_the_fs_policy() {
        let deny = |path: &str| Err(format!("'{path}' is outside the fs scope"));
        let file = absolute("secret.txt");
        let error = parse_drag_request(&json!({ "files": [file] }), deny).unwrap_err();
        assert!(error.contains("outside the fs scope"), "{error}");

        let only_files = |path: &str| {
            if path.contains("icon") {
                Err("icon denied".to_string())
            } else {
                Ok(PathBuf::from(path))
            }
        };
        let args = json!({ "files": [absolute("a.txt")], "iconPath": absolute("icon.png") });
        let error = parse_drag_request(&args, only_files).unwrap_err();
        assert!(error.contains("icon denied"), "{error}");
    }

    #[test]
    fn icon_bytes_are_bounded_bytes() {
        let file = absolute("a.txt");
        let request =
            parse_drag_request(&json!({ "files": [file], "iconBytes": [137, 80] }), allow).unwrap();
        assert_eq!(request.icon, DragIcon::Bytes(vec![137, 80]));
        let error =
            parse_drag_request(&json!({ "files": [file], "iconBytes": [256] }), allow).unwrap_err();
        assert!(error.contains("0-255"), "{error}");
        let args = json!({ "files": [file], "iconBytes": [1], "iconPath": file });
        assert!(parse_drag_request(&args, allow).is_err());
    }
}
