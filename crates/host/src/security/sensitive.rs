//! Built-in filesystem deny list: credential stores, browser profiles and the
//! host's own security state.
//!
//! The list applies to every renderer / plugin path check (`fs.*`, the
//! `asset://` scope, `security.checkFs` and every API that authorizes a path)
//! before the capability roots, the `fs.allow` rules and the runtime grants
//! are consulted. Nothing widens it: not an allow rule, not a dialog grant,
//! not `security.allowPath`. The trusted backend uses Bun's own filesystem
//! APIs and is not restricted by it.

use std::{path::PathBuf, sync::LazyLock};

use tynd_security_policy::{literal_glob, literal_subtree_glob, Scope};

static SENSITIVE: LazyLock<Scope> = LazyLock::new(|| {
    let mut deny = vec!["**/.netrc".to_string(), "**/_netrc".to_string()];
    for location in locations() {
        for spelling in spellings(&location) {
            deny.push(literal_glob(&spelling));
            deny.push(literal_subtree_glob(&spelling));
        }
    }
    Scope::new(Vec::new(), deny)
});

/// Reject `path` (lexical or kernel-resolved spelling) when it is, or lies
/// under, a protected location.
pub(crate) fn check(path: &std::path::Path) -> Result<(), String> {
    let Some(text) = path.to_str() else {
        return Ok(());
    };
    if SENSITIVE.denies_fs(text) {
        return Err(format!(
            "fs: '{}' is a protected location (credentials, browser profiles or Tynd security \
             state) that renderers and plugins can never reach",
            path.display()
        ));
    }
    Ok(())
}

/// The lexical spelling, plus the kernel-resolved one when it differs (a
/// symlinked or 8.3-named home directory).
fn spellings(location: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(text) = location.to_str() {
        out.push(text.to_string());
    }
    if let Some(resolved) = dunce::canonicalize(location)
        .ok()
        .and_then(|resolved| resolved.to_str().map(str::to_string))
    {
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    out
}

fn locations() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(security) = crate::security::grants::security_dir() {
        out.push(security);
    }
    let Some(home) = dirs::home_dir() else {
        return out;
    };
    for relative in [
        ".ssh",
        ".gnupg",
        ".aws",
        ".azure",
        ".kube",
        ".docker/config.json",
        ".config/gcloud",
        ".config/gh",
        ".git-credentials",
        ".config/git/credentials",
        ".npmrc",
        ".pypirc",
        ".cargo/credentials",
        ".cargo/credentials.toml",
        ".password-store",
    ] {
        out.push(home.join(relative));
    }
    platform_locations(&home, &mut out);
    out.extend(env_locations(|name| std::env::var_os(name)));
    out
}

/// Credential stores relocated through the environment, read once when the
/// deny list is first built (at startup).
fn env_locations(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let absolute = |value: std::ffi::OsString| {
        let path = PathBuf::from(value);
        if path.as_os_str().is_empty() {
            None
        } else if path.is_absolute() {
            Some(path)
        } else {
            std::env::current_dir().ok().map(|cwd| cwd.join(path))
        }
        // A filesystem root would deny everything: not a credential store.
        .filter(|path| path.parent().is_some())
    };
    let mut out = Vec::new();
    for name in [
        "GNUPGHOME",
        "AWS_SHARED_CREDENTIALS_FILE",
        "AWS_CONFIG_FILE",
        "NPM_CONFIG_USERCONFIG",
        "npm_config_userconfig",
    ] {
        out.extend(var(name).and_then(absolute));
    }
    if let Some(list) = var("KUBECONFIG") {
        out.extend(std::env::split_paths(&list).filter_map(|p| absolute(p.into_os_string())));
    }
    if let Some(docker) = var("DOCKER_CONFIG").and_then(absolute) {
        out.push(docker.join("config.json"));
    }
    if let Some(cargo) = var("CARGO_HOME").and_then(absolute) {
        out.push(cargo.join("credentials"));
        out.push(cargo.join("credentials.toml"));
    }
    out
}

#[cfg(windows)]
fn platform_locations(_home: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Some(roaming) = dirs::data_dir() {
        for relative in [
            "Microsoft/Credentials",
            "Microsoft/Protect",
            "Microsoft/Crypto",
            "Microsoft/SystemCertificates",
            "Mozilla/Firefox",
            "gnupg",
            "gcloud",
            "GitHub CLI",
        ] {
            out.push(roaming.join(relative));
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        for relative in [
            "Microsoft/Credentials",
            "Microsoft/Vault",
            "Google/Chrome/User Data",
            "Google/Chrome Beta/User Data",
            "Chromium/User Data",
            "Microsoft/Edge/User Data",
            "BraveSoftware/Brave-Browser/User Data",
            "Mozilla/Firefox",
        ] {
            out.push(local.join(relative));
        }
    }
}

#[cfg(target_os = "macos")]
fn platform_locations(home: &std::path::Path, out: &mut Vec<PathBuf>) {
    let library = home.join("Library");
    for relative in [
        "Keychains",
        "Cookies",
        "Safari",
        "Containers/com.apple.Safari",
        "Application Support/Google/Chrome",
        "Application Support/Chromium",
        "Application Support/Microsoft Edge",
        "Application Support/BraveSoftware",
        "Application Support/Firefox",
    ] {
        out.push(library.join(relative));
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_locations(home: &std::path::Path, out: &mut Vec<PathBuf>) {
    let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
    for base in [home.join(".config"), config] {
        for relative in [
            "google-chrome",
            "google-chrome-beta",
            "chromium",
            "microsoft-edge",
            "BraveSoftware",
        ] {
            out.push(base.join(relative));
        }
    }
    for base in [home.join(".local/share"), data] {
        out.push(base.join("keyrings"));
    }
    for relative in [".mozilla", "snap/firefox", ".var/app/org.mozilla.firefox"] {
        out.push(home.join(relative));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_stores_are_protected() {
        let home = dirs::home_dir().expect("home dir");
        for denied in [
            home.join(".ssh"),
            home.join(".ssh/id_ed25519"),
            home.join(".gnupg/private-keys-v1.d/key"),
            home.join(".aws/credentials"),
            home.join(".kube/config"),
            home.join(".docker/config.json"),
            home.join(".git-credentials"),
            home.join(".config/git/credentials"),
            home.join(".npmrc"),
            home.join(".pypirc"),
            home.join(".cargo/credentials"),
            home.join(".cargo/credentials.toml"),
            home.join(".config/gh/hosts.yml"),
            home.join(".password-store/bank.gpg"),
            home.join(".netrc"),
            home.join("_netrc"),
            home.join("projects/app/.netrc"),
        ] {
            assert!(
                check(&denied).is_err(),
                "{} must be denied",
                denied.display()
            );
        }
        for allowed in [
            home.join(".docker/other.json"),
            home.join(".cargo/registry/cache"),
            home.join(".sshx/file"),
            home.join("Documents/notes.txt"),
        ] {
            assert!(
                check(&allowed).is_ok(),
                "{} must not be denied",
                allowed.display()
            );
        }
    }

    #[test]
    fn env_relocated_stores_are_resolved() {
        let base = std::env::temp_dir().join("tynd-env-stores");
        let kube = std::env::join_paths([base.join("kube-a"), base.join("kube-b")]).expect("join");
        let found = env_locations(|name| match name {
            "GNUPGHOME" => Some(base.join("gnupg").into_os_string()),
            "AWS_SHARED_CREDENTIALS_FILE" => Some(base.join("aws-creds").into_os_string()),
            "AWS_CONFIG_FILE" => Some(base.join("aws-config").into_os_string()),
            "npm_config_userconfig" => Some(base.join("npmrc").into_os_string()),
            "KUBECONFIG" => Some(kube.clone()),
            "DOCKER_CONFIG" => Some(base.join("docker").into_os_string()),
            "CARGO_HOME" => Some(base.join("cargo").into_os_string()),
            "NPM_CONFIG_USERCONFIG" => Some(std::ffi::OsString::new()),
            _ => None,
        });
        for expected in [
            base.join("gnupg"),
            base.join("aws-creds"),
            base.join("aws-config"),
            base.join("npmrc"),
            base.join("kube-a"),
            base.join("kube-b"),
            base.join("docker/config.json"),
            base.join("cargo/credentials"),
            base.join("cargo/credentials.toml"),
        ] {
            assert!(found.contains(&expected), "{}", expected.display());
        }
        assert_eq!(found.len(), 9, "an empty variable adds nothing");
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert_eq!(
            env_locations(|name| (name == "GNUPGHOME").then(|| root.into())),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn browser_profiles_and_keychains_are_protected() {
        let home = dirs::home_dir().expect("home dir");
        let denied: Vec<PathBuf> = if cfg!(windows) {
            let roaming = dirs::data_dir().expect("roaming");
            let local = dirs::data_local_dir().expect("local");
            vec![
                roaming.join("Microsoft/Credentials/ABC"),
                roaming.join("Microsoft/Protect/S-1-5/key"),
                local.join("Google/Chrome/User Data/Default/Cookies"),
                local.join("Microsoft/Edge/User Data/Default/Login Data"),
                roaming.join("Mozilla/Firefox/Profiles/x/logins.json"),
            ]
        } else if cfg!(target_os = "macos") {
            vec![
                home.join("Library/Keychains/login.keychain-db"),
                home.join("Library/Application Support/Google/Chrome/Default/Cookies"),
                home.join("Library/Application Support/Firefox/Profiles/x"),
                home.join("Library/Safari/History.db"),
            ]
        } else {
            vec![
                home.join(".local/share/keyrings/login.keyring"),
                home.join(".config/google-chrome/Default/Cookies"),
                home.join(".mozilla/firefox/x/logins.json"),
            ]
        };
        for path in denied {
            assert!(check(&path).is_err(), "{} must be denied", path.display());
        }
    }

    #[test]
    fn the_grants_file_is_protected() {
        let security = crate::security::grants::security_dir().expect("data dir");
        assert!(check(&security.join("grants.json")).is_err());
        assert!(check(&security).is_err());
        let app_data = security.parent().expect("app data dir");
        if cfg!(any(windows, target_os = "macos")) {
            assert!(check(&app_data.join("SECURITY").join("grants.json")).is_err());
            assert!(check(&app_data.join("security").join("GRANTS.JSON")).is_err());
        }
        if cfg!(windows) {
            for variant in [
                "security.\\grants.json",
                "security. \\grants.json",
                "security\\grants.json.",
                "security\\grants.json::$DATA",
            ] {
                assert!(check(&app_data.join(variant)).is_err(), "{variant}");
            }
        }
        assert!(check(&app_data.join("settings.json")).is_ok());
    }
}
