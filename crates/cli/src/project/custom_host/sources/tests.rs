use std::collections::BTreeSet;

use serde_json::json;

use super::*;
use crate::project::custom_host::{
    fingerprint::{host_build_fingerprint, seed_cargo_lock},
    manifest::render_custom_cargo_manifest,
    test_support::{installed_host_sources, is_cache_key, toml_path_value},
};

#[test]
fn host_sources_follow_the_app_tynd_range() {
    let manifest = json!({ "devDependencies": { "@tynd/cli": "^1.2.0" } });
    assert_eq!(tynd_version_range(Some(&manifest)), "^1.2.0");
    let manifest = json!({ "dependencies": { "@tynd/core": "~1.0.3" } });
    assert_eq!(tynd_version_range(Some(&manifest)), "~1.0.3");
    assert_eq!(tynd_version_range(None), format!("^{}", crate::VERSION));
}

#[test]
fn host_sources_ignore_non_semver_tynd_ranges() {
    let fallback = format!("^{}", crate::VERSION);
    for range in [
        "workspace:*",
        "file:../tynd",
        ">=1.0.0 <2.0.0",
        "^1.0.0 --global",
        "latest",
        "https://example.com/x.tgz",
    ] {
        let manifest = json!({ "devDependencies": { "@tynd/cli": range } });
        assert_eq!(tynd_version_range(Some(&manifest)), fallback, "{range}");
    }
    let manifest = json!({ "devDependencies": { "@tynd/cli": "1.0.0" } });
    assert_eq!(tynd_version_range(Some(&manifest)), "1.0.0");
}

#[test]
fn host_sources_range_ignores_the_removed_host_package() {
    let manifest = json!({ "dependencies": { "@tynd/host": "^0.9.0" } });
    assert_eq!(
        tynd_version_range(Some(&manifest)),
        format!("^{}", crate::VERSION)
    );
}

/// An installed `@tynd/host-src` with a complete enough `native` tree for
/// `rust_source_paths`, plus build output that is not a source.
fn installed_host_tree(root: &Path, version: &str) -> PathBuf {
    let native = installed_host_sources(root, version);
    for (file, content) in [
        ("Cargo.toml", "[package]\nname = \"tynd-host\"\n"),
        ("Cargo.lock", "version = 4\n"),
        ("src/main.rs", "fn main() {}\n"),
        ("runtime/Cargo.toml", "[package]\nname = \"tynd-runtime\"\n"),
        ("vendor/tynd-plugin-sdk/Cargo.toml", "[package]\n"),
        ("target/debug/build-output", "stale"),
    ] {
        let path = native.join(file);
        fs::create_dir_all(path.parent().expect("parent")).expect("create source directory");
        fs::write(path, content).expect("write source file");
    }
    native
}

#[test]
fn installed_host_sources_are_compiled_from_one_cache_copy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let first = installed_host_tree(&dir.path().join("one"), "1.0.0");
    let second = installed_host_tree(&dir.path().join("two"), "1.0.0");

    let copy = cached_host_sources(&cache, first.clone()).expect("copy sources");
    assert_eq!(copy.version.as_deref(), Some("1.0.0"));
    let key = copy
        .root
        .strip_prefix(cache.join("host/src"))
        .expect("copy under host/src")
        .to_str()
        .expect("utf-8 key");
    assert!(is_cache_key(key), "{key}");
    assert!(cache.join("host/.tynd-cache").is_file());
    assert_eq!(
        fs::read_to_string(copy.root.join("src/main.rs")).expect("read copy"),
        "fn main() {}\n"
    );
    assert!(!copy.root.join("target").exists());

    // Another project with the same package compiles the same directory, and
    // the existing copy is reused as is.
    fs::write(copy.root.join("witness"), "kept").expect("write witness");
    let again = cached_host_sources(&cache, second).expect("reuse copy");
    assert_eq!(again.root, copy.root);
    assert!(copy.root.join("witness").is_file());
    assert_eq!(
        fs::read_dir(cache.join("host/src"))
            .expect("read host/src")
            .count(),
        1
    );

    // The manifest, the lockfile seed and the fingerprint use the copy.
    let paths = rust_source_paths(&copy.root).expect("source paths");
    let manifest = render_custom_cargo_manifest(
        "demo",
        "demo_0123abcd",
        &paths,
        &BTreeSet::new(),
        &[],
        &[],
        true,
    );
    let parsed: toml::Table = toml::from_str(&manifest).expect("generated manifest parses");
    for (dependency, directory) in [
        ("tynd-host", copy.root.clone()),
        ("tynd-runtime", copy.root.join("runtime")),
    ] {
        assert_eq!(
            parsed["dependencies"][dependency]["path"].as_str(),
            Some(toml_path_value(&directory).as_str()),
            "{dependency}"
        );
    }
    assert!(!manifest.contains("node_modules"), "{manifest}");
    let build_dir = dir.path().join("build");
    fs::create_dir_all(&build_dir).expect("create build dir");
    seed_cargo_lock(&build_dir, &copy.root).expect("seed");
    assert_eq!(
        fs::read_to_string(build_dir.join("Cargo.lock")).expect("lock"),
        "version = 4\n"
    );
    assert!(
        host_build_fingerprint(&copy, &manifest, "", &[], None, "rustc")
            .expect("fingerprint")
            .is_some()
    );
}

#[test]
fn host_sources_copy_follows_the_package_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let native = installed_host_tree(dir.path(), "1.0.0");
    let root = |native: &Path| {
        cached_host_sources(&cache, native.to_path_buf())
            .expect("copy sources")
            .root
    };
    let base = root(&native);
    assert_eq!(base, root(&native));

    // Build output is not a source.
    fs::write(native.join("target/debug/other"), "x").expect("write build output");
    assert_eq!(base, root(&native));

    // A source edit of the same size is a different copy; the first one is
    // left untouched.
    fs::write(native.join("src/main.rs"), "fn main() {;\n").expect("edit source");
    let edited = root(&native);
    assert_ne!(base, edited);
    assert_eq!(
        fs::read_to_string(base.join("src/main.rs")).expect("read first copy"),
        "fn main() {}\n"
    );
    assert_eq!(
        fs::read_to_string(edited.join("src/main.rs")).expect("read second copy"),
        "fn main() {;\n"
    );

    fs::write(native.join("Cargo.lock"), "version = 3\n").expect("edit lockfile");
    let relocked = root(&native);
    assert_ne!(edited, relocked);

    fs::write(native.join("src/extra.rs"), "").expect("add source");
    let extended = root(&native);
    assert_ne!(relocked, extended);

    installed_host_sources(dir.path(), "1.0.1");
    assert_ne!(extended, root(&native));
}

#[test]
fn editable_host_sources_are_compiled_in_place() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let checkout = dir.path().join("checkout");
    for manifest in ["crates/host/Cargo.toml", "crates/runtime/Cargo.toml"] {
        let path = checkout.join(manifest);
        fs::create_dir_all(path.parent().expect("parent")).expect("create crate");
        fs::write(path, "[package]\n").expect("write manifest");
    }
    let sources = cached_host_sources(&cache, checkout.clone()).expect("editable sources");
    assert_eq!(sources.root, checkout);
    assert!(sources.version.is_none());
    assert!(!cache.exists());

    let paths = rust_source_paths(&sources.root).expect("source paths");
    let manifest = render_custom_cargo_manifest(
        "demo",
        "demo_0123abcd",
        &paths,
        &BTreeSet::new(),
        &[],
        &[],
        true,
    );
    let parsed: toml::Table = toml::from_str(&manifest).expect("generated manifest parses");
    assert_eq!(
        parsed["dependencies"]["tynd-host"]["path"].as_str(),
        Some(toml_path_value(&checkout.join("crates/host")).as_str())
    );
}
