use super::*;
use crate::project::custom_host::{
    manifest::render_custom_cargo_manifest,
    test_support::{is_cache_key, native_plugin, plugin_crate_at, sources, toml_path_value},
};

#[test]
fn plugin_crates_are_compiled_from_content_addressed_cache_copies() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let build = dir.path().join("build");
    fs::create_dir_all(&build).expect("create build directory");
    let sdk = Path::new("/cache/host/src/key/vendor/tynd-plugin-sdk");
    let mut first = [native_plugin(&dir.path().join("one"), "pub fn a() {}")];
    let mut second = [native_plugin(&dir.path().join("two"), "pub fn a() {}")];
    for plugin in first.iter().chain(&second) {
        fs::write(
            plugin.native_dir.join("Cargo.toml"),
            "[package]\nname = \"acme-plugin\"\n\n[dependencies]\ntynd-plugin-sdk.workspace = true\n",
        )
        .expect("write plugin manifest");
        fs::write(plugin.native_dir.join("target/out"), "x").expect("write build output");
    }

    cache_native_plugins(&cache, &build, &mut first, sdk).expect("copy plugin");
    let crate_dir = first[0].crate_dir.clone();
    let copy = crate_dir.parent().expect("copy directory");
    assert_eq!(crate_dir.file_name(), Some(PLUGIN_COPY_CRATE_DIR.as_ref()));
    assert_eq!(copy.parent(), Some(cache.join("host/plugins").as_path()));
    let key = copy
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("acme-plugin-"))
        .expect("crate name prefix");
    assert!(is_cache_key(key), "{key}");
    assert!(cache.join("host/.tynd-cache").is_file());
    assert_eq!(
        fs::read_to_string(crate_dir.join("src/lib.rs")).expect("read copy"),
        "pub fn a() {}"
    );
    assert!(!crate_dir.join("target").exists());
    // The original crate is still what the fingerprint reads.
    assert!(first[0]
        .native_dir
        .starts_with(dir.path().join("one").canonicalize().expect("real")));

    // The copy carries the workspace the crate inherits from.
    let workspace: toml::Table =
        toml::from_str(&fs::read_to_string(copy.join("Cargo.toml")).expect("workspace manifest"))
            .expect("workspace manifest parses");
    assert_eq!(
        workspace["workspace"]["dependencies"]["tynd-plugin-sdk"]["path"].as_str(),
        Some("/cache/host/src/key/vendor/tynd-plugin-sdk")
    );

    // The app manifest links the copy.
    let manifest = render_custom_cargo_manifest(
        "demo",
        "demo_0123abcd",
        &sources(),
        &BTreeSet::new(),
        &first,
        &[],
        true,
    );
    let parsed: toml::Table = toml::from_str(&manifest).expect("generated manifest parses");
    assert_eq!(
        parsed["dependencies"]["acme-plugin"]["path"].as_str(),
        Some(toml_path_value(&crate_dir).as_str())
    );
    assert!(parsed["workspace"].get("members").is_none());

    // The same plugin installed in another project uses the same copy, as is.
    fs::write(copy.join("witness"), "kept").expect("write witness");
    cache_native_plugins(&cache, &build, &mut second, sdk).expect("reuse plugin copy");
    assert_eq!(second[0].crate_dir, crate_dir);
    assert!(copy.join("witness").is_file());

    // Edited content is a new copy; the first one is left untouched.
    fs::write(second[0].native_dir.join("src/lib.rs"), "pub fn b() {}").expect("edit plugin");
    cache_native_plugins(&cache, &build, &mut second, sdk).expect("copy edited plugin");
    assert_ne!(second[0].crate_dir, crate_dir);
    assert_eq!(
        fs::read_to_string(crate_dir.join("src/lib.rs")).expect("read first copy"),
        "pub fn a() {}"
    );
    assert_eq!(
        fs::read_to_string(second[0].crate_dir.join("src/lib.rs")).expect("read second copy"),
        "pub fn b() {}"
    );

    // So is a plugin SDK somewhere else (editable host sources).
    cache_native_plugins(
        &cache,
        &build,
        &mut first,
        Path::new("/checkout/crates/plugin-sdk"),
    )
    .expect("copy plugin for other sources");
    assert_ne!(first[0].crate_dir, crate_dir);
}

#[test]
fn editable_plugins_are_synchronized_into_a_stable_build_copy() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cache = dir.path().join("cache");
    let build = dir.path().join("build");
    fs::create_dir_all(&build).expect("create build directory");
    let sdk = Path::new("/cache/host/src/key/vendor/tynd-plugin-sdk");
    // Linked by `tynd plugin dev`: the canonical crate is outside node_modules.
    let mut plugins = [plugin_crate_at(
        &dir.path().join("acme-plugin/native"),
        "pub fn a() {}",
    )];
    let native = plugins[0].native_dir.clone();
    fs::write(
        native.join("Cargo.toml"),
        "[package]
name = \"acme-plugin\"

[dependencies]
tynd-plugin-sdk.workspace = true
",
    )
    .expect("write plugin manifest");
    fs::write(native.join("src/extra.rs"), "").expect("write extra source");
    // Left by earlier builds: a copy of a plugin no longer used.
    fs::create_dir_all(build.join("plugins/old_plugin/src")).expect("create stale copy");

    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("copy plugin");
    let crate_dir = plugins[0].crate_dir.clone();
    let copy = build.join("plugins/acme-plugin");
    assert_eq!(crate_dir, copy.join(PLUGIN_COPY_CRATE_DIR));
    assert!(!cache.join("host/plugins").exists());
    assert!(!build.join("plugins/old_plugin").exists());
    assert_eq!(
        fs::read_to_string(crate_dir.join("src/lib.rs")).expect("read copy"),
        "pub fn a() {}"
    );
    assert!(!crate_dir.join("target").exists());
    let workspace: toml::Table =
        toml::from_str(&fs::read_to_string(copy.join("Cargo.toml")).expect("workspace manifest"))
            .expect("workspace manifest parses");
    assert_eq!(
        workspace["workspace"]["dependencies"]["tynd-plugin-sdk"]["path"].as_str(),
        Some("/cache/host/src/key/vendor/tynd-plugin-sdk")
    );

    // An edit updates the same copy; a deleted file leaves it.
    fs::write(native.join("src/lib.rs"), "pub fn b() {}").expect("edit plugin");
    fs::remove_file(native.join("src/extra.rs")).expect("delete extra source");
    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("sync edited plugin");
    assert_eq!(plugins[0].crate_dir, crate_dir);
    assert_eq!(
        fs::read_to_string(crate_dir.join("src/lib.rs")).expect("read synced copy"),
        "pub fn b() {}"
    );
    assert!(!crate_dir.join("src/extra.rs").exists());
    assert!(copy.join("Cargo.toml").is_file());
    assert_eq!(
        fs::read_dir(build.join("plugins"))
            .expect("plugins")
            .count(),
        1
    );

    // Without editable plugins the directory goes away.
    cache_native_plugins(&cache, &build, &mut [], sdk).expect("no plugins");
    assert!(!build.join("plugins").exists());
}

/// An editable plugin (outside `node_modules`) under `root`, with the
/// directories its copy is synchronized into.
fn editable_plugin(root: &Path) -> (PathBuf, PathBuf, [NativeBuildPlugin; 1]) {
    let cache = root.join("cache");
    let build = root.join("build");
    fs::create_dir_all(&build).expect("create build directory");
    let plugins = [plugin_crate_at(
        &root.join("acme-plugin/native"),
        "pub fn a() {}",
    )];
    (cache, build, plugins)
}

const TEST_SDK: &str = "/cache/host/src/key/vendor/tynd-plugin-sdk";

fn entry_names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(directory)
        .expect("read directory")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn editable_plugin_copy_follows_a_case_only_rename() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (cache, build, mut plugins) = editable_plugin(dir.path());
    let native = plugins[0].native_dir.clone();
    fs::write(native.join("src/Utils.rs"), "pub fn u() {}").expect("write utils");
    let sdk = Path::new(TEST_SDK);
    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("copy plugin");

    fs::rename(native.join("src/Utils.rs"), native.join("src/utils.rs")).expect("rename utils");
    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("sync renamed plugin");
    let src = plugins[0].crate_dir.join("src");
    assert_eq!(entry_names(&src), ["lib.rs", "utils.rs"]);
    assert_eq!(
        fs::read_to_string(src.join("utils.rs")).expect("read renamed copy"),
        "pub fn u() {}"
    );
}

#[test]
fn editable_plugin_copy_follows_a_file_turned_directory_and_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (cache, build, mut plugins) = editable_plugin(dir.path());
    let native = plugins[0].native_dir.clone();
    let sdk = Path::new(TEST_SDK);
    fs::write(native.join("src/thing"), "file").expect("write thing file");
    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("copy plugin");
    let thing = plugins[0].crate_dir.join("src/thing");
    assert!(thing.is_file());

    fs::remove_file(native.join("src/thing")).expect("remove thing file");
    fs::create_dir_all(native.join("src/thing")).expect("create thing directory");
    fs::write(native.join("src/thing/inner.rs"), "inner").expect("write inner");
    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("sync file to directory");
    assert_eq!(
        fs::read_to_string(thing.join("inner.rs")).expect("read inner copy"),
        "inner"
    );

    fs::remove_dir_all(native.join("src/thing")).expect("remove thing directory");
    fs::write(native.join("src/thing"), "file again").expect("write thing file again");
    cache_native_plugins(&cache, &build, &mut plugins, sdk).expect("sync directory to file");
    assert_eq!(
        fs::read_to_string(&thing).expect("read thing copy"),
        "file again"
    );
}

#[test]
fn editable_plugin_copy_replaces_a_symlinked_crate_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (cache, build, mut plugins) = editable_plugin(dir.path());
    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).expect("create outside directory");
    let copy = build.join("plugins/acme-plugin");
    fs::create_dir_all(&copy).expect("create copy directory");
    let link = copy.join(PLUGIN_COPY_CRATE_DIR);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, &link);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&outside, &link);
    if linked.is_err() {
        // Creating symlinks is not permitted here (Windows without
        // developer mode).
        return;
    }

    cache_native_plugins(&cache, &build, &mut plugins, Path::new(TEST_SDK)).expect("copy plugin");
    assert!(fs::read_dir(&outside).expect("outside").next().is_none());
    let crate_type = fs::symlink_metadata(&link)
        .expect("crate directory")
        .file_type();
    assert!(crate_type.is_dir() && !crate_type.is_symlink());
    assert_eq!(
        fs::read_to_string(link.join("src/lib.rs")).expect("read copy"),
        "pub fn a() {}"
    );
}
