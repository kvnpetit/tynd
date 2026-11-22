//! The app's own directories: `<data>/<app>`, `<config>/<app>`,
//! `<cache>/<app>`, `<temp>/<app>` and the log directory.
//!
//! `<app>` is the app identifier (`bundle.identifier`, for example
//! `com.example.app`), delivered at startup through the trusted boot path:
//! the `app.identifier` pack entry in a packaged build, the
//! `TYND_APP_IDENTIFIER` variable set by `tynd dev` / `tynd start` otherwise
//! (`tynd-runtime` selects the source). Without one it falls back to the
//! executable name.
//!
//! Every user of these folders (store, log, window state, updater state,
//! persisted grants, the default filesystem scope, `os.app*Dir`, and the
//! runtime's crash reports through [`crash_dir`]) goes through [`app_dir`], so
//! they cannot diverge.

use std::{path::PathBuf, sync::OnceLock};

/// Resolved directory name. Set by [`set_identifier`] during boot, otherwise
/// filled with the executable-name fallback on first use.
static NAME: OnceLock<String> = OnceLock::new();

/// Fix the app identifier the app's directories are named after. Call once,
/// during boot, before anything resolves an app directory.
///
/// # Errors
/// When `identifier` is not a valid directory name
/// ([`tynd_pack_format::is_valid_app_identifier`]) or when the name was
/// already fixed (a second call, or a directory resolved earlier).
pub fn set_identifier(identifier: &str) -> Result<(), String> {
    if !tynd_pack_format::is_valid_app_identifier(identifier) {
        return Err(format!(
            "app identifier {identifier:?} is not a valid directory name; \
             falling back to the executable name"
        ));
    }
    NAME.set(identifier.to_string()).map_err(|_| {
        format!(
            "app identifier {identifier:?} arrived after the app directories were resolved as {:?}",
            name()
        )
    })
}

/// Name of the per-app directories: the app identifier, else the executable's
/// file stem, else `tynd-app`.
pub fn name() -> &'static str {
    NAME.get_or_init(|| {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .filter(|stem| !stem.is_empty() && !stem.starts_with('.'))
            .unwrap_or_else(|| "tynd-app".to_string())
    })
}

/// One of the app's own directories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppDir {
    /// `<data dir>/<app>`.
    Data,
    /// `<config dir>/<app>`.
    Config,
    /// `<cache dir>/<app>`.
    Cache,
    /// `<temp dir>/<app>`.
    Temp,
    /// `<cache dir>/<app>/logs` (data, then temp, when the OS has no cache dir).
    Log,
    /// `<local data dir>/<app>/crashes` (default `crashReporter` directory).
    Crashes,
}

impl AppDir {
    /// The renderer-reachable roots of the default filesystem scope.
    pub(crate) const SCOPE_ROOTS: [Self; 4] = [Self::Data, Self::Config, Self::Cache, Self::Temp];

    /// Wire suffix used in error messages (`os.appDataDir`).
    fn method(self) -> &'static str {
        match self {
            Self::Data => "appDataDir",
            Self::Config => "appConfigDir",
            Self::Cache => "appCacheDir",
            Self::Temp => "appTempDir",
            Self::Log => "appLogDir",
            Self::Crashes => "crashDir",
        }
    }
}

/// Path of `kind`, not created. `None` when the OS reports no base directory
/// for it (`Temp` and `Log` always resolve).
pub(crate) fn app_dir(kind: AppDir) -> Option<PathBuf> {
    let app = name();
    match kind {
        AppDir::Data => dirs::data_dir().map(|base| base.join(app)),
        AppDir::Config => dirs::config_dir().map(|base| base.join(app)),
        AppDir::Cache => dirs::cache_dir().map(|base| base.join(app)),
        AppDir::Temp => Some(std::env::temp_dir().join(app)),
        AppDir::Log => Some(
            app_dir(AppDir::Cache)
                .or_else(|| app_dir(AppDir::Data))
                .unwrap_or_else(|| std::env::temp_dir().join(app))
                .join("logs"),
        ),
        AppDir::Crashes => dirs::data_local_dir().map(|base| base.join(app).join("crashes")),
    }
}

/// `<local data dir>/<app>/crashes`, not created: the default directory of
/// the runtime's crash reports. `None` when the OS has no local data dir.
#[must_use]
pub fn crash_dir() -> Option<PathBuf> {
    app_dir(AppDir::Crashes)
}

/// Path of `kind`, created if missing.
///
/// # Errors
/// When the OS has no base directory for `kind` or creating it fails.
pub(crate) fn ensure_app_dir(kind: AppDir) -> Result<PathBuf, String> {
    let method = kind.method();
    let dir = app_dir(kind)
        .ok_or_else(|| format!("os.{method}: the system reports no base directory for it"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("os.{method}: create '{}': {error}", dir.display()))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dir_is_named_after_the_app() {
        for kind in [
            AppDir::Data,
            AppDir::Config,
            AppDir::Cache,
            AppDir::Temp,
            AppDir::Log,
            AppDir::Crashes,
        ] {
            let Some(dir) = app_dir(kind) else { continue };
            assert!(
                dir.components().any(|part| part.as_os_str() == name()),
                "{kind:?}: {}",
                dir.display()
            );
        }
        assert_eq!(
            app_dir(AppDir::Temp),
            Some(std::env::temp_dir().join(name()))
        );
    }

    #[test]
    fn crash_dir_is_under_the_local_data_dir() {
        assert_eq!(
            crash_dir(),
            dirs::data_local_dir().map(|base| base.join(name()).join("crashes"))
        );
    }

    #[test]
    fn log_dir_lives_inside_a_scope_root() {
        let log = app_dir(AppDir::Log).expect("log dir");
        assert!(AppDir::SCOPE_ROOTS
            .iter()
            .filter_map(|root| app_dir(*root))
            .any(|root| log.starts_with(root)));
    }

    #[test]
    fn invalid_identifiers_are_refused_before_fixing_the_name() {
        assert!(set_identifier("../escape").is_err());
        assert!(set_identifier("").is_err());
    }

    #[test]
    fn a_late_identifier_is_refused() {
        let _ = name();
        let error = set_identifier("com.example.late").expect_err("name already fixed");
        assert!(error.contains("after the app directories"), "{error}");
    }

    #[test]
    fn ensure_creates_the_directory() {
        let dir = ensure_app_dir(AppDir::Temp).expect("temp app dir");
        assert!(dir.is_dir());
    }
}
