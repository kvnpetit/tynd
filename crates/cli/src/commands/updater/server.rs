//! `tynd updater server`: scaffolds a hosted update server or serves the
//! release directory locally.

use anyhow::{bail, Result};
use serde_json::json;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::args::DeployProvider;

use crate::fsx::{absolute, atomic_write};
use crate::output::{display_path, step, success};

const MAX_UPDATE_FILE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug)]
pub(in crate::commands) struct UpdateServerRequest {
    pub(in crate::commands) cwd: PathBuf,
    pub(in crate::commands) serve: bool,
    pub(in crate::commands) port: u16,
    pub(in crate::commands) provider: Option<DeployProvider>,
    pub(in crate::commands) public_url: Option<String>,
    pub(in crate::commands) public_key: Option<String>,
}

pub(in crate::commands) fn update_server(request: &UpdateServerRequest) -> Result<()> {
    if request.serve {
        return serve_updates(request);
    }
    let provider = request
        .provider
        .unwrap_or(DeployProvider::Cloudflare)
        .as_str();
    scaffold_update_server(request, provider)
}

fn scaffold_update_server(request: &UpdateServerRequest, provider: &str) -> Result<()> {
    if !matches!(provider, "cloudflare" | "vercel" | "deno") {
        bail!("unknown provider {provider}; expected cloudflare, vercel, or deno");
    }
    let project_root = absolute(&request.cwd)?;
    let destination = project_root
        .join(".tynd")
        .join("update-server")
        .join(provider);
    fs::create_dir_all(&destination)?;
    let config = crate::config::load(&project_root, None).ok();
    let app_id = config
        .as_ref()
        .and_then(|loaded| loaded.value.pointer("/bundle/identifier"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("com.example.app");
    let public_key = request
        .public_key
        .as_deref()
        .or_else(|| {
            config
                .as_ref()
                .and_then(|loaded| configured_public_key(&loaded.value))
        })
        .unwrap_or("");
    let manifest_url = request
        .public_url
        .as_deref()
        .unwrap_or("https://updates.example.com/manifest.json");
    render_update_server_templates(
        provider,
        &destination,
        &[
            ("${publicKey}", public_key),
            ("${manifestUrl}", manifest_url),
            ("${appId}", app_id),
            ("${compatibilityDate}", "2026-07-30"),
        ],
    )?;
    let settings = json!({
        "appId": app_id,
        "manifestUrl": manifest_url,
        "publicKey": public_key,
    });
    atomic_write(
        &destination.join("tynd-update.json"),
        format!("{}\n", serde_json::to_string_pretty(&settings)?),
    )?;
    success(format!("Update server -> {}", display_path(&destination)));
    if provider != "deno" {
        step(
            "without the AUTH_TOKEN secret the endpoints are public; manifests are signed, so \
             clients still reject tampered updates",
        );
    }
    Ok(())
}

/// First entry of `updater.publicKeys`.
fn configured_public_key(config: &serde_json::Value) -> Option<&str> {
    config
        .pointer("/updater/publicKeys/0")
        .and_then(serde_json::Value::as_str)
}

/// Update-server templates compiled into the binary, so `updater server`
/// works however `tynd` was launched: `(provider, path under the provider
/// directory, contents)`. The sources live in `crates/cli/templates/`.
const UPDATE_SERVER_TEMPLATES: &[(&str, &str, &str)] = &[
    (
        "cloudflare",
        "worker.ts.tpl",
        include_str!("../../../templates/update-server/cloudflare/worker.ts.tpl"),
    ),
    (
        "cloudflare",
        "wrangler.toml.tpl",
        include_str!("../../../templates/update-server/cloudflare/wrangler.toml.tpl"),
    ),
    (
        "deno",
        "mod.ts.tpl",
        include_str!("../../../templates/update-server/deno/mod.ts.tpl"),
    ),
    (
        "vercel",
        "vercel.json.tpl",
        include_str!("../../../templates/update-server/vercel/vercel.json.tpl"),
    ),
    (
        "vercel",
        "api/manifest.ts.tpl",
        include_str!("../../../templates/update-server/vercel/api/manifest.ts.tpl"),
    ),
];

/// Writes the embedded templates of `provider` under `destination`, without
/// their `.tpl` suffix and with every placeholder replaced.
fn render_update_server_templates(
    provider: &str,
    destination: &Path,
    replacements: &[(&str, &str)],
) -> Result<()> {
    for (_, relative, template) in UPDATE_SERVER_TEMPLATES
        .iter()
        .filter(|(owner, ..)| *owner == provider)
    {
        let output = destination.join(relative.strip_suffix(".tpl").unwrap_or(relative));
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut rendered = (*template).to_owned();
        for (placeholder, value) in replacements {
            rendered = rendered.replace(placeholder, &escape_template_value(value));
        }
        atomic_write(&output, rendered)?;
    }
    Ok(())
}

fn escape_template_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}

fn serve_updates(request: &UpdateServerRequest) -> Result<()> {
    let project_root = absolute(&request.cwd)?;
    for required in [
        project_root.join("release/publish/manifest.json"),
        project_root.join("release/publish/manifest.json.sig"),
    ] {
        if !required.is_file() {
            bail!(
                "local update server requires {}; run `tynd updater publish <version> --target manifest-only` first",
                required.display()
            );
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", request.port))?;
    listener.set_nonblocking(true)?;
    let running = Arc::new(AtomicBool::new(true));
    let signal = running.clone();
    ctrlc::set_handler(move || signal.store(false, Ordering::Release))?;
    success(format!("Update server: http://127.0.0.1:{}", request.port));
    while running.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = handle_update_request(stream, request) {
                    eprintln!("update request: {error}");
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(30));
            },
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn handle_update_request(mut stream: TcpStream, request: &UpdateServerRequest) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let url_path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let path = url_path.split(['?', '#']).next().unwrap_or("/");
    let release = absolute(&request.cwd)?.join("release");
    let requested = match path {
        "/" | "/manifest.json" => release.join("publish/manifest.json"),
        "/manifest.json.sig" => release.join("publish/manifest.json.sig"),
        value => {
            let raw = value.trim_start_matches('/');
            if !safe_update_relative_path(raw) {
                return respond(&mut stream, 400, "text/plain", b"bad path\n");
            }
            release.join(raw)
        },
    };
    if !requested.is_file() {
        return respond(&mut stream, 404, "text/plain", b"not found\n");
    }
    let release_root = match fs::canonicalize(&release) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return respond(&mut stream, 404, "text/plain", b"not found\n");
        },
        Err(error) => return Err(error.into()),
    };
    let requested = match fs::canonicalize(&requested) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return respond(&mut stream, 404, "text/plain", b"not found\n");
        },
        Err(error) => return Err(error.into()),
    };
    if !requested.starts_with(&release_root) {
        return respond(&mut stream, 400, "text/plain", b"bad path\n");
    }
    let file = fs::File::open(&requested)?;
    if file.metadata()?.len() > MAX_UPDATE_FILE_BYTES {
        return respond(&mut stream, 413, "text/plain", b"file too large\n");
    }
    let mut body = Vec::new();
    file.take(MAX_UPDATE_FILE_BYTES + 1)
        .read_to_end(&mut body)?;
    if body.len() as u64 > MAX_UPDATE_FILE_BYTES {
        return respond(&mut stream, 413, "text/plain", b"file too large\n");
    }
    let content_type = if requested.extension().and_then(|value| value.to_str()) == Some("json") {
        "application/json"
    } else {
        "application/octet-stream"
    };
    respond(&mut stream, 200, content_type, &body)
}

fn safe_update_relative_path(raw: &str) -> bool {
    if raw.is_empty() || raw.contains('\\') || raw.contains(':') {
        return false;
    }
    let path = Path::new(raw);
    if path.is_absolute()
        || raw
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return false;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        413 => "Payload Too Large",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{configured_public_key, render_update_server_templates, UPDATE_SERVER_TEMPLATES};
    use serde_json::json;

    #[test]
    fn public_key_is_the_first_of_the_list() {
        let list_only = json!({ "updater": { "publicKeys": ["B", "C"] } });
        assert_eq!(configured_public_key(&list_only), Some("B"));

        assert_eq!(configured_public_key(&json!({ "updater": {} })), None);
        assert_eq!(configured_public_key(&json!({})), None);
    }

    #[test]
    fn update_server_templates_come_from_the_binary() {
        let dir = tempfile::tempdir().expect("tempdir");
        for provider in ["cloudflare", "deno", "vercel"] {
            let destination = dir.path().join(provider);
            render_update_server_templates(
                provider,
                &destination,
                &[
                    ("${publicKey}", "KEY\"1"),
                    ("${manifestUrl}", "https://u.test/m.json"),
                ],
            )
            .expect("render");
            assert!(!destination.join("README.md").exists(), "no stub README");
            let written = walkdir::WalkDir::new(&destination)
                .into_iter()
                .map(|entry| entry.expect("entry"))
                .filter(|entry| entry.file_type().is_file())
                .collect::<Vec<_>>();
            assert!(!written.is_empty(), "{provider} has templates");
            for entry in written {
                let name = entry.file_name().to_string_lossy();
                assert!(!name.ends_with(".tpl"), "{name} keeps its .tpl suffix");
                let body = std::fs::read_to_string(entry.path()).expect("read");
                assert!(!body.contains("${publicKey}"), "{name} keeps a placeholder");
            }
        }
        let manifest = std::fs::read_to_string(dir.path().join("vercel/api/manifest.ts"))
            .expect("nested template");
        assert!(manifest.contains("https://u.test/m.json"));
    }

    #[test]
    fn every_template_source_is_embedded() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates/update-server");
        let mut on_disk = walkdir::WalkDir::new(&root)
            .into_iter()
            .map(|entry| entry.expect("entry"))
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| {
                let relative = entry.path().strip_prefix(&root).expect("relative");
                let body = std::fs::read_to_string(entry.path()).expect("read template");
                (relative.to_string_lossy().replace('\\', "/"), body)
            })
            .collect::<Vec<_>>();
        on_disk.sort();
        let mut embedded = UPDATE_SERVER_TEMPLATES
            .iter()
            .map(|(provider, relative, body)| {
                (format!("{provider}/{relative}"), (*body).to_owned())
            })
            .collect::<Vec<_>>();
        embedded.sort();
        assert_eq!(embedded, on_disk);
    }
}
