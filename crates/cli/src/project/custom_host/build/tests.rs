use serde_json::json;

use super::*;
use crate::project::custom_host::{plugins::is_cargo_package_name, test_support::parse};

#[test]
fn bin_name_is_unique_per_project_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = dir.path().join("one/app");
    let second = dir.path().join("two/app");
    fs::create_dir_all(&first).expect("create project");
    fs::create_dir_all(&second).expect("create project");
    let name = host_bin_name("app", &first);
    assert_eq!(name, host_bin_name("app", &first));
    assert_ne!(name, host_bin_name("app", &second));
    let suffix = name.strip_prefix("app_").expect("app name prefix");
    assert_eq!(suffix.len(), 8);
    assert!(is_cargo_package_name(&name), "{name}");
}

#[test]
fn built_binary_is_copied_to_the_project_and_found_there() {
    let dir = tempfile::tempdir().expect("tempdir");
    let project = dir.path().join("app");
    let build_dir = prepare_custom_build_directory(&project_dir(&project)).expect("build dir");
    let shared = dir.path().join("cache/host/key/debug");
    fs::create_dir_all(&shared).expect("create shared target");
    let built = shared.join("app_0123abcd");
    fs::write(&built, "first").expect("write built binary");

    let destination = host_binary_path(&project, false).expect("host path");
    assert!(crate::project::resolve_host_binary(&project, false).is_err());
    install_host_binary(&built, &destination, &build_dir).expect("install");
    assert_eq!(
        crate::project::resolve_host_binary(&project, false).expect("resolved"),
        destination
    );
    assert_eq!(fs::read(&destination).expect("read copy"), b"first");
    assert!(destination.starts_with(project.join(".tynd/build/bin/debug")));

    // A rebuild replaces the copy and leaves no temporary file behind.
    fs::write(&built, "second").expect("rewrite built binary");
    install_host_binary(&built, &destination, &build_dir).expect("reinstall");
    assert_eq!(fs::read(&destination).expect("read copy"), b"second");
    let entries = fs::read_dir(destination.parent().expect("bin dir"))
        .expect("read bin dir")
        .count();
    assert_eq!(entries, 1);
    // The release profile has its own copy.
    assert!(crate::project::resolve_host_binary(&project, true).is_err());
}

fn project_dir(project: &Path) -> PathBuf {
    fs::create_dir_all(project).expect("create project");
    project.to_path_buf()
}

#[test]
fn custom_host_always_enables_every_core_api() {
    let apis = collect_host_apis(vec!["dialog".to_owned()], &[]).expect("known APIs");
    for core in CORE_HOST_APIS {
        assert!(apis.contains(*core), "missing core API {core}");
    }
    assert!(apis.contains("dialog"));
    assert!(collect_host_apis(vec!["nope".to_owned()], &[]).is_err());
}

#[test]
fn dev_builds_use_the_cargo_dev_profile() {
    let config = parse(json!({}));
    let build_dir = Path::new("/app/.tynd/build");
    let target_dir = Path::new("/cache/host/key");
    let cargo = HostCargo {
        build_dir,
        target_dir,
        host_triple: "x86_64-unknown-linux-gnu",
    };
    let dev = cargo_build_command(&cargo, &config, false).redacted();
    let release = cargo_build_command(&cargo, &config, true).redacted();
    assert!(!dev.contains("--release"), "{dev}");
    assert!(release.contains("--release"), "{release}");
}

#[test]
fn cargo_build_pins_the_shared_target_directory() {
    let config = parse(json!({}));
    let build_dir = Path::new("/app/.tynd/build");
    let target_dir = Path::new("/cache/host/key");
    let cargo = HostCargo {
        build_dir,
        target_dir,
        host_triple: "aarch64-apple-darwin",
    };
    let command = cargo_build_command(&cargo, &config, false);
    let value = |flag: &str| {
        command
            .args
            .iter()
            .position(|arg| arg == flag)
            .and_then(|index| command.args.get(index + 1))
            .unwrap_or_else(|| panic!("{flag} value"))
    };
    assert_eq!(Path::new(value("--target-dir")), target_dir);
    assert_eq!(value("--target"), "aarch64-apple-darwin");
    assert_eq!(
        command.env.get(OsStr::new("CARGO_TARGET_DIR")),
        Some(&Some(target_dir.as_os_str().to_owned()))
    );
    for variable in ["CARGO_BUILD_TARGET", "CARGO_BUILD_TARGET_DIR"] {
        assert_eq!(
            command.env.get(OsStr::new(variable)),
            Some(&None),
            "{variable}"
        );
    }
}

#[test]
fn built_binary_is_read_from_the_host_triple_directory() {
    let target_dir = Path::new("/cache/host/key");
    let cargo = HostCargo {
        build_dir: Path::new("/app/.tynd/build"),
        target_dir,
        host_triple: "x86_64-pc-windows-msvc",
    };
    assert_eq!(
        built_host_binary(&cargo, false, "demo.exe"),
        target_dir.join("x86_64-pc-windows-msvc/debug/demo.exe")
    );
    assert_eq!(
        built_host_binary(&cargo, true, "demo.exe"),
        target_dir.join("x86_64-pc-windows-msvc/release/demo.exe")
    );
}

#[test]
fn flamegraph_shares_the_build_command_settings() {
    let config = parse(json!({}));
    let target_dir = Path::new("/cache/host/key");
    let cargo = HostCargo {
        build_dir: Path::new("/app/.tynd/build"),
        target_dir,
        host_triple: "x86_64-unknown-linux-gnu",
    };
    let build = cargo_build_command(&cargo, &config, true);
    let profile = host_cargo_command(vec![OsString::from("flamegraph")], &cargo, &config);
    assert_eq!(profile.env, build.env);
    assert_eq!(profile.cwd, build.cwd);
    let tail = |command: &CommandSpec| command.args[command.args.len() - 4..].to_vec();
    assert_eq!(tail(&profile), tail(&build));
}

#[test]
fn legacy_build_target_directory_is_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let legacy = dir.path().join("target/debug");
    fs::create_dir_all(&legacy).expect("create legacy target");
    remove_legacy_target_dir(dir.path());
    assert!(!dir.path().join("target").exists());
    remove_legacy_target_dir(dir.path());
}

#[test]
fn macos_host_builds_target_the_minimum_macos_unless_set() {
    assert_eq!(
        macos_deployment_target("aarch64-apple-darwin", None),
        Some(tynd_bundle_contract::MACOS_MINIMUM_SYSTEM_VERSION)
    );
    assert_eq!(
        macos_deployment_target("x86_64-apple-darwin", Some(OsStr::new(""))),
        Some("14.0")
    );
    assert_eq!(
        macos_deployment_target("aarch64-apple-darwin", Some(OsStr::new("15.0"))),
        None
    );
    assert_eq!(
        macos_deployment_target("x86_64-pc-windows-msvc", None),
        None
    );
    assert_eq!(
        macos_deployment_target("x86_64-unknown-linux-gnu", None),
        None
    );
}
