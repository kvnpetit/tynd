//! `tynd report`: a zipped diagnostic bundle without source contents.

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use std::{fs, io::Write, path::PathBuf};

use super::updater::epoch_seconds;
use crate::output::{display_path, success, write_json};

#[derive(Debug)]
pub(super) struct ReportRequest {
    pub(super) cwd: PathBuf,
    pub(super) out: Option<PathBuf>,
    pub(super) json: bool,
}

#[derive(Debug, Serialize)]
struct ReportOutput {
    path: String,
    files: usize,
}

pub(super) fn report(request: &ReportRequest) -> Result<PathBuf> {
    const PROJECT_FILES: &[&str] = &[
        "package.json",
        "Cargo.toml",
        "tynd.config.ts",
        "tynd.config.mts",
        "tynd.config.cts",
        "tynd.config.js",
        "tynd.config.mjs",
        "tynd.config.cjs",
        "tynd.config.json",
        "tynd.config.jsonc",
        "tynd.config.toml",
        "tynd.config.yaml",
        "tynd.config.yml",
    ];

    let output_dir = request
        .out
        .clone()
        .unwrap_or_else(|| request.cwd.join(".tynd").join("reports"));
    fs::create_dir_all(&output_dir)?;
    let archive = output_dir.join(format!("tynd-report-{}.zip", epoch_seconds()));
    let file = fs::File::create(&archive)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let present_files = PROJECT_FILES
        .iter()
        .copied()
        .filter(|name| request.cwd.join(name).is_file())
        .collect::<Vec<_>>();
    let project_summary = json!({
        "presentFiles": present_files,
        "sourceContentsIncluded": false,
    });
    zip.start_file("project-summary.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(&project_summary)?)?;

    let project_directory = request
        .cwd
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("project");
    let environment = json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "tynd": crate::VERSION,
        "projectDirectory": project_directory,
    });
    zip.start_file("environment.json", options)?;
    zip.write_all(&serde_json::to_vec_pretty(&environment)?)?;
    zip.finish()?;

    const FILE_COUNT: usize = 2;
    if request.json {
        write_json(&ReportOutput {
            path: archive.display().to_string(),
            files: FILE_COUNT,
        })?;
    } else {
        success(format!("Report -> {}", display_path(&archive)));
    }
    Ok(archive)
}
