use serde_json::json;

use super::*;
use crate::project::custom_host::{
    sources::{cached_host_sources, installed_host_sources_version, HOST_SOURCES_PACKAGE},
    test_support::{installed_host_sources, native_plugin},
};

/// `source_root` as the build sees it when it is used in place.
fn host_sources_at(source_root: &Path) -> HostSources {
    HostSources {
        root: source_root.to_path_buf(),
        version: installed_host_sources_version(source_root),
    }
}

fn fingerprint(source_root: &Path, manifest: &str, plugins: &[NativeBuildPlugin]) -> String {
    host_build_fingerprint(
        &host_sources_at(source_root),
        manifest,
        "fn main() {}",
        plugins,
        None,
        "rustc 1.95.0",
    )
    .expect("fingerprint")
    .expect("installed sources have a fingerprint")
}

#[test]
fn dev_fingerprint_is_stable_for_unchanged_inputs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let native = installed_host_sources(dir.path(), "1.0.0");
    let plugins = [native_plugin(dir.path(), "pub fn a() {}")];
    assert_eq!(
        fingerprint(&native, "manifest", &plugins),
        fingerprint(&native, "manifest", &plugins)
    );
}

#[test]
fn dev_fingerprint_follows_every_build_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    let native = installed_host_sources(dir.path(), "1.0.0");
    let plugins = [native_plugin(dir.path(), "pub fn a() {}")];
    let base = fingerprint(&native, "manifest", &plugins);

    // APIs, Cargo features and plugin features all land in the manifest.
    assert_ne!(
        base,
        fingerprint(&native, "manifest with api-menu", &plugins)
    );
    assert_ne!(base, fingerprint(&native, "manifest", &[]));
    let pinned = host_build_fingerprint(
        &host_sources_at(&native),
        "manifest",
        "fn main() {}",
        &plugins,
        Some("k"),
        "rustc 1.95.0",
    )
    .expect("fingerprint");
    assert_ne!(Some(&base), pinned.as_ref());
    // The compiler (version line plus RUSTFLAGS) is an input.
    let other_compiler = host_build_fingerprint(
        &host_sources_at(&native),
        "manifest",
        "fn main() {}",
        &plugins,
        None,
        "rustc 1.96.0",
    )
    .expect("fingerprint");
    assert_ne!(Some(&base), other_compiler.as_ref());
    // So is the host sources' lockfile.
    fs::write(native.join("Cargo.lock"), "version = 4").expect("write host-src lock");
    let locked = fingerprint(&native, "manifest", &plugins);
    assert_ne!(base, locked);
    fs::remove_file(native.join("Cargo.lock")).expect("remove host-src lock");

    // Build output inside the plugin crate is not an input.
    fs::write(plugins[0].native_dir.join("target/out"), "x").expect("write build output");
    assert_eq!(base, fingerprint(&native, "manifest", &plugins));

    fs::write(plugins[0].native_dir.join("src/lib.rs"), "pub fn b() {}").expect("edit plugin");
    let edited = fingerprint(&native, "manifest", &plugins);
    assert_ne!(base, edited);

    installed_host_sources(dir.path(), "1.0.1");
    assert_ne!(edited, fingerprint(&native, "manifest", &plugins));
}

#[test]
fn editable_host_sources_always_go_through_cargo() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A Tynd checkout: sources outside node_modules.
    let checkout = dir.path().join("checkout");
    fs::create_dir_all(&checkout).expect("create checkout");
    let sources =
        cached_host_sources(&dir.path().join("cache"), checkout.clone()).expect("editable sources");
    assert!(sources.version.is_none());
    assert!(
        host_build_fingerprint(&sources, "manifest", "", &[], None, "")
            .expect("fingerprint")
            .is_none()
    );
    // A linked @tynd/host-src resolves outside node_modules too.
    let linked = dir.path().join("packages/host-src/native");
    fs::create_dir_all(&linked).expect("create linked sources");
    fs::write(
        dir.path().join("packages/host-src/package.json"),
        json!({ "name": HOST_SOURCES_PACKAGE, "version": "1.0.0" }).to_string(),
    )
    .expect("write manifest");
    assert!(installed_host_sources_version(&linked).is_none());
}

#[test]
fn cargo_lock_is_seeded_from_the_host_sources() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sources = dir.path().join("native");
    let build_dir = dir.path().join("build");
    fs::create_dir_all(&sources).expect("create sources");
    fs::create_dir_all(&build_dir).expect("create build dir");
    let lock = build_dir.join("Cargo.lock");

    // No source lockfile: cargo resolves on its own.
    seed_cargo_lock(&build_dir, &sources).expect("seed");
    assert!(!lock.exists());

    fs::write(sources.join("Cargo.lock"), "v1").expect("write source lock");
    seed_cargo_lock(&build_dir, &sources).expect("seed");
    assert_eq!(fs::read_to_string(&lock).expect("lock"), "v1");

    // Cargo's additions (app and plugin deps) survive while the sources match.
    fs::write(&lock, "v1 + plugin deps").expect("cargo updates lock");
    seed_cargo_lock(&build_dir, &sources).expect("seed");
    assert_eq!(fs::read_to_string(&lock).expect("lock"), "v1 + plugin deps");

    // New host sources replace it.
    fs::write(sources.join("Cargo.lock"), "v2").expect("update source lock");
    seed_cargo_lock(&build_dir, &sources).expect("seed");
    assert_eq!(fs::read_to_string(&lock).expect("lock"), "v2");

    // A deleted app lockfile is seeded again.
    fs::remove_file(&lock).expect("remove lock");
    seed_cargo_lock(&build_dir, &sources).expect("seed");
    assert_eq!(fs::read_to_string(&lock).expect("lock"), "v2");
}
