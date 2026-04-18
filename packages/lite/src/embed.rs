//! Embedded-assets loader for the lite runtime.
//!
//! Lite pack entries: `bundle.js`, `frontend/<path>`, `icon.{png,ico}`,
//! `sidecar/<name>`. See `tynd_host::embed` for the wire format.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;

use tynd_host::embed::{finalize_sidecars, prepare_extract_dir, PackReader};

pub(crate) struct EmbeddedAssets {
    pub bundle_path: String,
    pub frontend_dir: String,
    pub icon_path: Option<String>,
}

pub(crate) fn try_load_embedded() -> Option<EmbeddedAssets> {
    let mut pack = PackReader::open()?;
    let paths = prepare_extract_dir()?;

    let mut bundle_path_opt: Option<String> = None;
    let mut frontend_dir_opt: Option<String> = None;
    let mut icon_path_opt: Option<String> = None;
    let mut sidecars_pending: Vec<(String, PathBuf)> = Vec::new();

    while let Some((rel, data_len)) = pack.next_header() {
        let dest: PathBuf = if rel == "bundle.js" {
            let p = paths.bundle_dir.join("bundle.js");
            bundle_path_opt = Some(p.to_string_lossy().into_owned());
            p
        } else if let Some(rest) = rel.strip_prefix("frontend/") {
            let p = paths.frontend_dir.join(rest);
            if let Some(parent) = p.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if frontend_dir_opt.is_none() {
                frontend_dir_opt = Some(paths.frontend_dir.to_string_lossy().into_owned());
            }
            p
        } else if rel == "icon.png" || rel == "icon.ico" {
            let p = paths.temp_dir.join(&rel);
            icon_path_opt = Some(p.to_string_lossy().into_owned());
            p
        } else if let Some(rest) = rel.strip_prefix("sidecar/") {
            let p = paths.temp_dir.join("sidecar").join(rest);
            if let Some(parent) = p.parent() {
                let _ = fs::create_dir_all(parent);
            }
            sidecars_pending.push((rest.to_string(), p.clone()));
            p
        } else {
            pack.skip(data_len).ok()?;
            continue;
        };

        let out = File::create(&dest).ok()?;
        let mut writer = BufWriter::new(out);
        std::io::copy(&mut pack.reader().take(data_len), &mut writer).ok()?;
        writer.flush().ok()?;
    }

    let bundle_path = bundle_path_opt.unwrap_or_else(|| {
        tynd_host::tynd_log!("Embedded pack is missing bundle.js — rebuild with `tynd build`");
        tynd_host::cleanup::run();
        std::process::exit(1);
    });
    let frontend_dir = frontend_dir_opt.unwrap_or_else(|| {
        tynd_host::tynd_log!("Embedded pack is missing frontend files — rebuild with `tynd build`");
        tynd_host::cleanup::run();
        std::process::exit(1);
    });

    finalize_sidecars(&sidecars_pending);

    Some(EmbeddedAssets {
        bundle_path,
        frontend_dir,
        icon_path: icon_path_opt,
    })
}
