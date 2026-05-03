//! Native bridges for the Node compat polyfills (process, os, fs).
//!
//! Each bridge is a tiny shim over `tynd_host::os` so we share one impl
//! with the OS APIs in `@tynd/core/client`. Kept synchronous because the
//! polyfills these back are themselves sync (path/process metadata, fs
//! sync calls, hostname/cpus/memory probes).
//!
//! Errors throw a JS exception via `rquickjs::Error::new_from_js_message`
//! so callers see `try { fs.readFileSync('x') } catch (e) { ... }`.
//!
//! NOTE: fs operations on disk paths are gated by the same OS sandbox
//! `tynd_host::os::fs` enforces. There's no extra ACL layer here — the
//! lite runtime is single-tenant: the running JS bundle *is* the user.
use base64::{engine::general_purpose::STANDARD, Engine};
use rquickjs::{Ctx, Exception, Function, Object, Value};

fn throw<'js>(ctx: &Ctx<'js>, msg: impl Into<String>) -> rquickjs::Error {
    Exception::throw_message(ctx, &msg.into())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn register<'js>(ctx: &Ctx<'js>, g: &Object<'js>) -> rquickjs::Result<()> {
    register_process(ctx, g)?;
    register_os(ctx, g)?;
    register_fs(ctx, g)?;
    Ok(())
}

fn register_process<'js>(ctx: &Ctx<'js>, g: &Object<'js>) -> rquickjs::Result<()> {
    g.set(
        "__tynd_platform__",
        Function::new(ctx.clone(), || -> String {
            // Match what Node.js reports across OSes — the JS polyfill
            // branches on `"win32" | "darwin" | "linux"`.
            match std::env::consts::OS {
                "macos" => "darwin".into(),
                "windows" => "win32".into(),
                other => other.into(),
            }
        })?,
    )?;
    g.set(
        "__tynd_arch__",
        Function::new(ctx.clone(), || -> String {
            match std::env::consts::ARCH {
                "x86_64" => "x64".into(),
                "aarch64" => "arm64".into(),
                other => other.into(),
            }
        })?,
    )?;
    g.set(
        "__tynd_pid__",
        Function::new(ctx.clone(), || -> u32 { std::process::id() })?,
    )?;
    g.set(
        "__tynd_cwd__",
        Function::new(ctx.clone(), || -> String {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "/".to_string())
        })?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_chdir__",
        Function::new(ctx.clone(), move |path: String| -> rquickjs::Result<()> {
            std::env::set_current_dir(&path).map_err(|e| throw(&cctx, format!("chdir: {e}")))
        })?,
    )?;
    g.set(
        "__tynd_env_get__",
        Function::new(ctx.clone(), |key: String| -> Option<String> {
            std::env::var(&key).ok()
        })?,
    )?;
    g.set(
        "__tynd_env_keys__",
        Function::new(ctx.clone(), || -> Vec<String> {
            std::env::vars().map(|(k, _)| k).collect()
        })?,
    )?;
    g.set(
        "__tynd_env_set__",
        Function::new(ctx.clone(), |key: String, val: String| {
            // SAFETY: setting env vars is technically unsound on POSIX
            // multi-threaded processes (libc strdup, no lock). Lite runs
            // user JS on a single thread and other host threads only
            // read `env` during init; the audit bar matches Node's own
            // `process.env[k] = v`, which is also unsound.
            unsafe { std::env::set_var(&key, &val) };
        })?,
    )?;
    g.set(
        "__tynd_env_del__",
        Function::new(ctx.clone(), |key: String| {
            unsafe { std::env::remove_var(&key) };
        })?,
    )?;
    g.set(
        "__tynd_argv__",
        Function::new(ctx.clone(), || -> Vec<String> { std::env::args().collect() })?,
    )?;
    g.set(
        "__tynd_exit__",
        Function::new(ctx.clone(), |code: i32| -> () {
            tynd_host::cleanup::run();
            std::process::exit(code);
        })?,
    )?;
    g.set(
        "__tynd_hrtime_ms__",
        Function::new(ctx.clone(), || -> f64 {
            // Wall-clock ms since epoch — Node `process.hrtime()` is a
            // monotonic delta but the JS shim only consumes the value as
            // an absolute reference, so wall-clock is fine.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0)
        })?,
    )?;
    g.set(
        "__tynd_uptime_ms__",
        Function::new(ctx.clone(), || -> f64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64() * 1000.0)
                .unwrap_or(0.0)
        })?,
    )?;
    Ok(())
}

fn register_os<'js>(ctx: &Ctx<'js>, g: &Object<'js>) -> rquickjs::Result<()> {
    // Reuse host-rs `OsInfoApi` so we share the same probe backed by the
    // workspace's `dirs` / `hostname` / `os_info` deps without re-pinning
    // them in the lite crate.
    g.set(
        "__tynd_os_homedir__",
        Function::new(ctx.clone(), || -> Option<String> {
            match tynd_host::os::os_info::OsInfoApi::home_dir() {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            }
        })?,
    )?;
    g.set(
        "__tynd_os_tmpdir__",
        Function::new(ctx.clone(), || -> String {
            match tynd_host::os::os_info::OsInfoApi::tmp_dir() {
                serde_json::Value::String(s) => s,
                _ => "/tmp".into(),
            }
        })?,
    )?;
    g.set(
        "__tynd_os_hostname__",
        Function::new(ctx.clone(), || -> Option<String> {
            match tynd_host::os::os_info::OsInfoApi::hostname() {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            }
        })?,
    )?;
    g.set(
        "__tynd_os_type__",
        Function::new(ctx.clone(), || -> String {
            match std::env::consts::OS {
                "macos" => "Darwin".into(),
                "windows" => "Windows_NT".into(),
                "linux" => "Linux".into(),
                other => other.into(),
            }
        })?,
    )?;
    g.set(
        "__tynd_os_release__",
        Function::new(ctx.clone(), || -> String {
            // OsInfoApi::info() returns { platform, arch, family, version }.
            match tynd_host::os::os_info::OsInfoApi::info() {
                serde_json::Value::Object(o) => o
                    .get("version")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_else(|| "0.0.0".into()),
                _ => "0.0.0".into(),
            }
        })?,
    )?;
    g.set(
        "__tynd_os_cpus__",
        Function::new(ctx.clone(), || -> u32 {
            std::thread::available_parallelism()
                .map(|n| n.get() as u32)
                .unwrap_or(1)
        })?,
    )?;
    // Memory + uptime — use sysinfo through tynd_host::os::system_stats so
    // we share the same probe with the os.system_stats RPC.
    g.set(
        "__tynd_os_totalmem__",
        Function::new(ctx.clone(), || -> f64 {
            use serde_json::Value;
            let v = tynd_host::os::system_stats::SystemStatsApi::memory();
            if let Value::Object(o) = v {
                if let Some(t) = o.get("total").and_then(Value::as_u64) {
                    return t as f64;
                }
            }
            0.0
        })?,
    )?;
    g.set(
        "__tynd_os_freemem__",
        Function::new(ctx.clone(), || -> f64 {
            use serde_json::Value;
            let v = tynd_host::os::system_stats::SystemStatsApi::memory();
            if let Value::Object(o) = v {
                if let Some(f) = o.get("free").and_then(Value::as_u64) {
                    return f as f64;
                }
            }
            0.0
        })?,
    )?;
    g.set(
        "__tynd_os_uptime__",
        Function::new(ctx.clone(), || -> f64 {
            use serde_json::Value;
            let v = tynd_host::os::system_stats::SystemStatsApi::uptime();
            if let Value::Number(n) = v {
                return n.as_f64().unwrap_or(0.0);
            }
            0.0
        })?,
    )?;
    g.set(
        "__tynd_os_loadavg__",
        Function::new(ctx.clone(), || -> Vec<f64> {
            use serde_json::Value;
            let v = tynd_host::os::system_stats::SystemStatsApi::cpu();
            if let Value::Object(o) = v {
                if let Some(arr) = o.get("loadavg").and_then(Value::as_array) {
                    return arr.iter().filter_map(Value::as_f64).collect();
                }
            }
            vec![0.0, 0.0, 0.0]
        })?,
    )?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn register_fs<'js>(ctx: &Ctx<'js>, g: &Object<'js>) -> rquickjs::Result<()> {
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_read_text__",
        Function::new(ctx.clone(), move |path: String| -> rquickjs::Result<String> {
            std::fs::read_to_string(&path).map_err(|e| throw(&cctx, format!("readFileSync: {e}")))
        })?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_read_bytes__",
        Function::new(ctx.clone(), move |path: String| -> rquickjs::Result<String> {
            std::fs::read(&path)
                .map(|b| STANDARD.encode(b))
                .map_err(|e| throw(&cctx, format!("readFileSync: {e}")))
        })?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_write_text__",
        Function::new(
            ctx.clone(),
            move |path: String, text: String| -> rquickjs::Result<()> {
                std::fs::write(&path, text.as_bytes())
                    .map_err(|e| throw(&cctx, format!("writeFileSync: {e}")))
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_write_bytes__",
        Function::new(
            ctx.clone(),
            move |path: String, b64: String| -> rquickjs::Result<()> {
                let bytes = STANDARD
                    .decode(b64.as_bytes())
                    .map_err(|e| throw(&cctx, format!("writeFileSync: bad base64: {e}")))?;
                std::fs::write(&path, &bytes)
                    .map_err(|e| throw(&cctx, format!("writeFileSync: {e}")))
            },
        )?,
    )?;
    g.set(
        "__tynd_fs_exists__",
        Function::new(ctx.clone(), |path: String| -> bool {
            std::path::Path::new(&path).exists()
        })?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_stat__",
        Function::new(
            ctx.clone(),
            move |path: String| -> rquickjs::Result<Value<'js>> {
                let m = std::fs::metadata(&path)
                    .map_err(|e| throw(&cctx, format!("statSync: {e}")))?;
                let mtime_ms = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let obj = Object::new(cctx.clone())?;
                obj.set("size", m.len() as f64)?;
                obj.set("isFile", m.is_file())?;
                obj.set("isDirectory", m.is_dir())?;
                obj.set("isSymbolicLink", m.is_symlink())?;
                obj.set("mtimeMs", mtime_ms)?;
                obj.set("ctimeMs", mtime_ms)?;
                obj.set("atimeMs", mtime_ms)?;
                obj.set("birthtimeMs", mtime_ms)?;
                obj.set("mode", 0o644u32)?;
                obj.set("uid", 0u32)?;
                obj.set("gid", 0u32)?;
                obj.set("nlink", 1u32)?;
                obj.set("dev", 0u32)?;
                obj.set("ino", 0u32)?;
                obj.set("rdev", 0u32)?;
                obj.set("blksize", 4096u32)?;
                obj.set("blocks", (m.len() / 512) as f64)?;
                Ok(obj.into())
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_readdir__",
        Function::new(
            ctx.clone(),
            move |path: String| -> rquickjs::Result<Vec<String>> {
                let entries = std::fs::read_dir(&path)
                    .map_err(|e| throw(&cctx, format!("readdirSync: {e}")))?;
                let mut out = Vec::new();
                for ent in entries.flatten() {
                    out.push(ent.file_name().to_string_lossy().into_owned());
                }
                Ok(out)
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_mkdir__",
        Function::new(
            ctx.clone(),
            move |path: String, recursive: bool| -> rquickjs::Result<()> {
                if recursive {
                    std::fs::create_dir_all(&path).map_err(|e| throw(&cctx, format!("mkdirSync: {e}")))
                } else {
                    std::fs::create_dir(&path).map_err(|e| throw(&cctx, format!("mkdirSync: {e}")))
                }
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_rm__",
        Function::new(
            ctx.clone(),
            move |path: String, recursive: bool| -> rquickjs::Result<()> {
                let p = std::path::Path::new(&path);
                if !p.exists() {
                    return Ok(()); // mirror `force: true` semantics
                }
                let r = if p.is_dir() {
                    if recursive {
                        std::fs::remove_dir_all(p)
                    } else {
                        std::fs::remove_dir(p)
                    }
                } else {
                    std::fs::remove_file(p)
                };
                r.map_err(|e| throw(&cctx, format!("rmSync: {e}")))
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_rename__",
        Function::new(
            ctx.clone(),
            move |from: String, to: String| -> rquickjs::Result<()> {
                std::fs::rename(&from, &to).map_err(|e| throw(&cctx, format!("renameSync: {e}")))
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_copy__",
        Function::new(
            ctx.clone(),
            move |from: String, to: String| -> rquickjs::Result<()> {
                std::fs::copy(&from, &to)
                    .map(|_| ())
                    .map_err(|e| throw(&cctx, format!("copyFileSync: {e}")))
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_realpath__",
        Function::new(
            ctx.clone(),
            move |path: String| -> rquickjs::Result<String> {
                std::fs::canonicalize(&path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| throw(&cctx, format!("realpathSync: {e}")))
            },
        )?,
    )?;
    let cctx = ctx.clone();
    g.set(
        "__tynd_fs_chmod__",
        Function::new(
            ctx.clone(),
            move |path: String, mode: u32| -> rquickjs::Result<()> {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let perms = std::fs::Permissions::from_mode(mode);
                    std::fs::set_permissions(&path, perms)
                        .map_err(|e| throw(&cctx, format!("chmodSync: {e}")))
                }
                #[cfg(not(unix))]
                {
                    let _ = (path, mode, &cctx); // silence unused on windows
                    Ok(()) // no-op on Windows — Node's `chmod` ignores most bits there too
                }
            },
        )?,
    )?;
    Ok(())
}
