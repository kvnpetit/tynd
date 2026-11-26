use super::*;

#[test]
fn environment_override_wins_over_the_user_cache_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = cache_root_from(
        Some(dir.path().as_os_str().to_owned()),
        Some(PathBuf::from("/user/cache")),
    )
    .expect("cache root");
    assert_eq!(root, std::path::absolute(dir.path()).expect("absolute"));
}

#[test]
fn relative_override_is_made_absolute() {
    let root = cache_root_from(Some(OsString::from("relative-cache")), None).expect("root");
    assert!(root.is_absolute(), "{}", root.display());
    assert!(root.ends_with("relative-cache"));
}

#[cfg(windows)]
#[test]
fn override_is_normalized_to_native_separators() {
    let root =
        cache_root_from(Some(OsString::from("C:/tynd-cache/./nested")), None).expect("cache root");
    assert_eq!(root, PathBuf::from(r"C:\tynd-cache\nested"));
}

#[test]
fn default_cache_root_is_the_tynd_directory_of_the_user_cache() {
    let base = PathBuf::from("base");
    let expected = std::path::absolute(if cfg!(windows) {
        base.join("tynd").join("cache")
    } else {
        base.join("tynd")
    })
    .expect("absolute");
    assert_eq!(
        cache_root_from(None, Some(base.clone())).expect("cache root"),
        expected
    );
    // An empty override counts as unset.
    assert_eq!(
        cache_root_from(Some(OsString::new()), Some(base)).expect("cache root"),
        expected
    );
}

#[test]
fn missing_user_cache_directory_names_the_override() {
    let error = cache_root_from(None, None)
        .expect_err("no cache directory")
        .to_string();
    assert!(error.contains(CACHE_DIR_VARIABLE), "{error}");
}

#[test]
fn cache_key_follows_sources_compiler_and_host_triple() {
    let base = host_cache_key("host-src@1.0.0", "rustc 1.95.0", "x86_64-pc-windows-msvc");
    assert_eq!(base.len(), HOST_CACHE_KEY_LEN);
    assert_eq!(
        base,
        host_cache_key("host-src@1.0.0", "rustc 1.95.0", "x86_64-pc-windows-msvc")
    );
    assert_ne!(
        base,
        host_cache_key("host-src@1.0.1", "rustc 1.95.0", "x86_64-pc-windows-msvc")
    );
    assert_ne!(
        base,
        host_cache_key("host-src@1.0.0", "rustc 1.96.0", "x86_64-pc-windows-msvc")
    );
    assert_ne!(
        base,
        host_cache_key("host-src@1.0.0", "rustc 1.95.0", "aarch64-pc-windows-msvc")
    );
    // Field boundaries are part of the key.
    assert_ne!(host_cache_key("ab", "c", ""), host_cache_key("a", "bc", ""));
}

#[test]
fn target_directory_lives_under_the_host_cache() {
    let root = Path::new("cache");
    let target = host_target_dir(root, "0123456789ab");
    assert_eq!(target, root.join("host").join("0123456789ab"));
    assert!(target.starts_with(host_cache_dir_in(root)));
}

#[test]
fn removing_the_host_cache_keeps_the_rest_of_the_cache_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = host_cache_dir_in(dir.path());
    assert!(!remove_host_cache(&host, false).expect("nothing to remove"));

    let artifact = prepare_host_target_dir(dir.path(), "key")
        .expect("prepare target")
        .join("debug/app");
    fs::create_dir_all(artifact.parent().expect("profile dir")).expect("create target");
    fs::write(&artifact, "binary").expect("write artifact");
    fs::write(dir.path().join("other"), "kept").expect("write sibling");

    assert!(remove_host_cache(&host, true).expect("dry run"));
    assert!(artifact.is_file(), "dry run must not remove anything");
    assert!(remove_host_cache(&host, false).expect("remove"));
    assert!(!host.exists());
    assert!(dir.path().join("other").is_file());
}

#[test]
fn a_host_directory_without_the_marker_is_not_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = host_cache_dir_in(dir.path());
    fs::create_dir_all(host.join("data")).expect("create unrelated directory");
    let error = remove_host_cache(&host, false)
        .expect_err("must refuse")
        .to_string();
    assert!(error.contains(HOST_CACHE_MARKER), "{error}");
    assert!(!error.contains("  "), "{error}");
    assert!(error.contains("did not create it"), "{error}");
    assert!(host.join("data").is_dir());

    fs::remove_dir(host.join("data")).expect("remove unrelated directory");
    prepare_host_target_dir(dir.path(), "key").expect("prepare target");
    assert!(host.join(HOST_CACHE_MARKER).is_file());
    assert!(remove_host_cache(&host, false).expect("remove"));
    assert!(!host.exists());
}

#[test]
fn the_marker_is_not_written_into_unrelated_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = host_cache_dir_in(dir.path());
    fs::create_dir_all(&host).expect("create host directory");
    fs::write(host.join("notes.txt"), "user data").expect("write user file");
    let error = prepare_host_target_dir(dir.path(), "key")
        .expect_err("must refuse")
        .to_string();
    assert!(error.contains(CACHE_DIR_VARIABLE), "{error}");
    assert!(!host.join(HOST_CACHE_MARKER).exists());

    // An empty `host` directory is adopted.
    fs::remove_file(host.join("notes.txt")).expect("remove user file");
    prepare_host_target_dir(dir.path(), "key").expect("prepare target");
    assert!(host.join(HOST_CACHE_MARKER).is_file());
    // Once marked, a populated cache keeps working.
    fs::create_dir_all(host.join("key")).expect("create target");
    prepare_host_target_dir(dir.path(), "other").expect("prepare again");
}

#[test]
fn a_marker_written_by_another_process_meanwhile_is_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let host = host_cache_dir_in(dir.path());
    fs::create_dir_all(&host).expect("create host directory");
    let marker = host.join(HOST_CACHE_MARKER);
    // Our first check saw no marker; another process then created the
    // cache and started filling it.
    fs::write(&marker, "theirs").expect("write marker");
    fs::create_dir_all(host.join("src")).expect("create their copy directory");
    adopt_host_cache_dir(&host, &marker).expect("adopt the cache another process created");
    assert_eq!(fs::read_to_string(&marker).expect("read marker"), "theirs");
}

#[test]
fn stale_temporary_copies_are_removed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stale = dir.path().join(".tmp-crashed");
    fs::create_dir_all(stale.join("crate")).expect("create crashed copy");
    fs::create_dir_all(dir.path().join("acme_plugin-0123456789ab")).expect("create installed copy");
    fs::write(dir.path().join(".tmp-file"), "").expect("write temporary file");

    // A recent temporary directory may belong to a copy still running.
    remove_stale_temporaries(dir.path(), STALE_TEMPORARY_AGE);
    assert!(stale.is_dir());

    remove_stale_temporaries(dir.path(), std::time::Duration::ZERO);
    assert!(!stale.exists());
    assert!(dir.path().join("acme_plugin-0123456789ab").is_dir());
    assert!(dir.path().join(".tmp-file").is_file());
}

fn entries(directory: &Path) -> usize {
    fs::read_dir(directory).expect("read directory").count()
}

#[test]
fn cache_copies_live_under_the_host_cache() {
    let root = Path::new("cache");
    let host = host_cache_dir_in(root);
    assert_eq!(
        host_sources_copy_dir(root, "0123456789ab"),
        host.join("src").join("0123456789ab")
    );
    assert_eq!(
        plugin_copy_dir(root, "acme_plugin", "0123456789ab"),
        host.join("plugins").join("acme_plugin-0123456789ab")
    );
    assert_eq!(short_key(&blake3::hash(b"x")).len(), HOST_CACHE_KEY_LEN);
}

#[test]
fn a_cache_copy_is_installed_once_and_never_rewritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let destination = host_sources_copy_dir(dir.path(), "key");
    let parent = destination.parent().expect("src dir");
    install_cache_copy(dir.path(), &destination, |temporary| {
        assert_eq!(temporary.parent(), Some(parent));
        fs::write(temporary.join("file"), "first")?;
        Ok(())
    })
    .expect("install");
    assert_eq!(
        fs::read_to_string(destination.join("file")).expect("read copy"),
        "first"
    );
    assert!(host_cache_dir_in(dir.path())
        .join(HOST_CACHE_MARKER)
        .is_file());

    // An existing copy is reused: `fill` does not run again.
    install_cache_copy(dir.path(), &destination, |_| {
        panic!("an existing copy must be reused")
    })
    .expect("reuse");
    // Only the copy is left, no temporary directory.
    assert_eq!(entries(parent), 1);
}

#[test]
fn a_copy_installed_by_another_process_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    let destination = host_sources_copy_dir(dir.path(), "key");
    install_cache_copy(dir.path(), &destination, |temporary| {
        fs::write(temporary.join("file"), "ours")?;
        // Another process finishes the same copy while ours is written.
        fs::create_dir_all(&destination)?;
        fs::write(destination.join("file"), "theirs")?;
        Ok(())
    })
    .expect("install");
    assert_eq!(
        fs::read_to_string(destination.join("file")).expect("read copy"),
        "theirs"
    );
    assert_eq!(entries(destination.parent().expect("src dir")), 1);
}

#[test]
fn a_failed_copy_leaves_nothing_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let destination = host_sources_copy_dir(dir.path(), "key");
    let result = install_cache_copy(dir.path(), &destination, |temporary| {
        fs::write(temporary.join("file"), "partial")?;
        bail!("copy failed")
    });
    assert!(result.is_err());
    assert!(!destination.exists());
    assert_eq!(entries(destination.parent().expect("src dir")), 0);
}

#[test]
fn long_target_directories_warn_on_windows_only() {
    let short = Path::new(r"C:\c\host\0123456789ab");
    let long = PathBuf::from(format!(r"C:\{}\host\0123456789ab", "d".repeat(80)));
    assert!(long_target_dir_warning_for(short, true).is_none());
    assert!(long_target_dir_warning_for(&long, false).is_none());
    let warning = long_target_dir_warning_for(&long, true).expect("warning");
    assert!(warning.contains(CACHE_DIR_VARIABLE), "{warning}");
}
