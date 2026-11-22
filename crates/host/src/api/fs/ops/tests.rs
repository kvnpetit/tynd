//! Unit tests for the capability-confined FS operations. Every test works
//! inside a fresh `tempfile::tempdir()` (the OS temp dir is a default
//! capability root) and the security policy stays unconfigured, so only the
//! cap-std root confinement is exercised.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::*;

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn p(base: &Path, rel: &str) -> String {
    base.join(rel)
        .to_str()
        .expect("utf-8 temp path")
        .to_string()
}

/// A path that lexically sits outside every capability root: a fresh
/// directory name directly under the filesystem root of the temp dir.
fn out_of_scope(file: &str) -> String {
    let temp = std::env::temp_dir();
    let root: PathBuf = temp.ancestors().last().expect("fs root").to_path_buf();
    root.join("tynd-out-of-scope-test-dir")
        .join(file)
        .to_str()
        .unwrap()
        .to_string()
}

fn err_of(result: Result<Value, String>) -> String {
    result.expect_err("operation should fail")
}

#[test]
fn missing_path_argument_is_reported_by_name() {
    assert_eq!(err_of(read_text(&json!({}))), "fs: missing 'path' argument");
    assert_eq!(
        err_of(read_text(&json!({ "path": 5 }))),
        "fs: missing 'path' argument"
    );
    assert_eq!(
        err_of(rename(&json!({ "to": "x" }))),
        "fs: missing 'from' argument"
    );
}

#[test]
fn empty_path_is_rejected() {
    assert!(err_of(read_text(&json!({ "path": "" }))).contains("cannot be empty"));
}

#[test]
fn write_then_read_text_round_trips() {
    let dir = tmp();
    let file = p(dir.path(), "a.txt");
    assert_eq!(
        write_text(&json!({ "path": file, "content": "héllo" })).unwrap(),
        Value::Null
    );
    assert_eq!(read_text(&json!({ "path": file })).unwrap(), json!("héllo"));
    // Overwrite truncates.
    write_text(&json!({ "path": file, "content": "x" })).unwrap();
    assert_eq!(read_text(&json!({ "path": file })).unwrap(), json!("x"));
}

#[test]
fn write_text_requires_content() {
    let dir = tmp();
    let e = err_of(write_text(&json!({ "path": p(dir.path(), "a.txt") })));
    assert_eq!(e, "fs.writeText: missing 'content'");
}

#[test]
fn write_text_create_dirs_controls_parent_creation() {
    let dir = tmp();
    let nested = p(dir.path(), "x/y/z.txt");
    let e = err_of(write_text(&json!({ "path": nested, "content": "a" })));
    assert!(e.starts_with("fs.writeText("), "{e}");
    assert!(!dir.path().join("x").exists());

    write_text(&json!({ "path": nested, "content": "a", "createDirs": true })).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x/y/z.txt")).unwrap(),
        "a"
    );
}

#[test]
fn read_text_of_missing_file_names_the_path() {
    let dir = tmp();
    let file = p(dir.path(), "nope.txt");
    let e = err_of(read_text(&json!({ "path": file })));
    assert!(e.starts_with("fs.readText: metadata"), "{e}");
}

#[test]
fn read_text_rejects_directories_and_invalid_utf8() {
    let dir = tmp();
    let e = err_of(read_text(&json!({ "path": dir.path().to_str().unwrap() })));
    assert!(e.contains("is not a regular file"), "{e}");

    std::fs::write(dir.path().join("bin"), [0xff, 0xfe, 0x00]).unwrap();
    let e = err_of(read_text(&json!({ "path": p(dir.path(), "bin") })));
    assert!(e.starts_with("fs.readText("), "{e}");
}

#[test]
fn full_file_operations_are_capped() {
    let dir = tmp();
    let big = dir.path().join("big.bin");
    // Sparse on every mainstream FS: no 64 MiB write needed.
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_FULL_FILE_BYTES + 1)
        .unwrap();
    let big = big.to_str().unwrap();

    let e = err_of(read_text(&json!({ "path": big })));
    assert!(e.contains("capped at"), "{e}");

    let e = err_of(copy(
        &json!({ "from": big, "to": p(dir.path(), "copy.bin") }),
    ));
    assert!(e.contains("capped at"), "{e}");
    assert!(!dir.path().join("copy.bin").exists());

    std::fs::create_dir(dir.path().join("tree")).unwrap();
    std::fs::rename(dir.path().join("big.bin"), dir.path().join("tree/big.bin")).unwrap();
    let e = err_of(copy_dir(&json!({
        "from": p(dir.path(), "tree"),
        "to": p(dir.path(), "tree2"),
    })));
    assert!(e.contains("capped at"), "{e}");
}

#[test]
fn file_exactly_at_cap_is_allowed() {
    let dir = tmp();
    let at_cap = dir.path().join("cap.bin");
    std::fs::File::create(&at_cap)
        .unwrap()
        .set_len(MAX_FULL_FILE_BYTES)
        .unwrap();
    let dir_handle = Dir::open_ambient_dir(dir.path(), cap_std::ambient_authority()).unwrap();
    assert!(ensure_file_within_cap(&dir_handle, Path::new("cap.bin"), "t").is_ok());
}

#[test]
fn out_of_scope_paths_are_refused() {
    let outside = out_of_scope("x.txt");
    let e = err_of(read_text(&json!({ "path": outside })));
    assert!(e.contains("outside capability roots"), "{e}");
    let e = err_of(write_text(
        &json!({ "path": outside, "content": "a", "createDirs": true }),
    ));
    assert!(e.contains("outside capability roots"), "{e}");
    assert!(!Path::new(&outside).exists());
}

#[test]
fn dot_dot_traversal_out_of_roots_is_refused() {
    let dir = tmp();
    let depth = dir.path().components().count() + 2;
    let mut path = dir.path().to_path_buf();
    for _ in 0..depth {
        path.push("..");
    }
    path.push("tynd-out-of-scope-test-dir");
    path.push("x.txt");
    let e = err_of(write_text(
        &json!({ "path": path.to_str().unwrap(), "content": "a" }),
    ));
    assert!(e.contains("outside capability roots"), "{e}");
}

#[test]
fn dot_dot_inside_root_is_cleaned_lexically() {
    let dir = tmp();
    std::fs::write(dir.path().join("a.txt"), "ok").unwrap();
    let via_dots = p(dir.path(), "sub/../a.txt");
    // `sub` does not exist: lexical cleaning must not touch it.
    assert_eq!(
        read_text(&json!({ "path": via_dots })).unwrap(),
        json!("ok")
    );
}

#[cfg(windows)]
#[test]
fn unc_paths_are_refused_without_probing() {
    let e = err_of(read_text(
        &json!({ "path": r"\\tynd-nonexistent-host\share\x.txt" }),
    ));
    assert!(e.contains("outside capability roots"), "{e}");
}

#[test]
fn exists_collapses_out_of_scope_to_false() {
    let dir = tmp();
    std::fs::write(dir.path().join("a"), "").unwrap();
    assert_eq!(
        exists(&json!({ "path": p(dir.path(), "a") })).unwrap(),
        json!(true)
    );
    assert_eq!(
        exists(&json!({ "path": p(dir.path(), "b") })).unwrap(),
        json!(false)
    );
    assert_eq!(
        exists(&json!({ "path": out_of_scope("a") })).unwrap(),
        json!(false)
    );
    assert!(exists(&json!({})).is_err(), "missing arg is still an error");
}

#[test]
fn stat_reports_kind_size_and_mtime() {
    let dir = tmp();
    std::fs::write(dir.path().join("f"), "12345").unwrap();
    let s = stat(&json!({ "path": p(dir.path(), "f") })).unwrap();
    assert_eq!(s["size"], 5);
    assert_eq!(s["isFile"], true);
    assert_eq!(s["isDir"], false);
    assert!(s["mtime"].as_u64().unwrap() > 0);

    let s = stat(&json!({ "path": dir.path().to_str().unwrap() })).unwrap();
    assert_eq!(s["isDir"], true);

    let e = err_of(stat(&json!({ "path": p(dir.path(), "missing") })));
    assert!(e.starts_with("fs.stat("), "{e}");
}

#[test]
fn read_dir_lists_entries_with_types() {
    let dir = tmp();
    std::fs::write(dir.path().join("file.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let Value::Array(mut entries) =
        read_dir(&json!({ "path": dir.path().to_str().unwrap() })).unwrap()
    else {
        panic!("array expected");
    };
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["name"], "file.txt");
    assert_eq!(entries[0]["isFile"], true);
    assert_eq!(entries[1]["name"], "sub");
    assert_eq!(entries[1]["isDir"], true);

    let e = err_of(read_dir(&json!({ "path": p(dir.path(), "missing") })));
    assert!(e.starts_with("fs.readDir("), "{e}");
}

#[test]
fn mkdir_recursive_flag() {
    let dir = tmp();
    let nested = p(dir.path(), "a/b/c");
    assert!(err_of(mkdir(&json!({ "path": nested }))).starts_with("fs.mkdir("));
    mkdir(&json!({ "path": nested, "recursive": true })).unwrap();
    assert!(dir.path().join("a/b/c").is_dir());
    // Non-recursive on an existing dir fails; recursive is idempotent.
    assert!(mkdir(&json!({ "path": nested })).is_err());
    mkdir(&json!({ "path": nested, "recursive": true })).unwrap();
}

#[test]
fn remove_files_and_directories() {
    let dir = tmp();
    std::fs::create_dir_all(dir.path().join("d/inner")).unwrap();
    std::fs::write(dir.path().join("d/inner/f"), "x").unwrap();
    std::fs::write(dir.path().join("f"), "x").unwrap();

    remove(&json!({ "path": p(dir.path(), "f") })).unwrap();
    assert!(!dir.path().join("f").exists());

    let e = err_of(remove(&json!({ "path": p(dir.path(), "d") })));
    assert!(e.starts_with("fs.remove("), "{e}");
    assert!(
        dir.path().join("d/inner/f").exists(),
        "non-recursive must not delete"
    );

    remove(&json!({ "path": p(dir.path(), "d"), "recursive": true })).unwrap();
    assert!(!dir.path().join("d").exists());

    let e = err_of(remove(&json!({ "path": p(dir.path(), "gone") })));
    assert!(e.starts_with("fs.remove("), "{e}");
}

#[test]
fn rename_moves_and_reports_both_paths_on_error() {
    let dir = tmp();
    std::fs::write(dir.path().join("a"), "1").unwrap();
    rename(&json!({ "from": p(dir.path(), "a"), "to": p(dir.path(), "b") })).unwrap();
    assert!(!dir.path().join("a").exists());
    assert_eq!(std::fs::read_to_string(dir.path().join("b")).unwrap(), "1");

    let from = p(dir.path(), "missing");
    let to = p(dir.path(), "c");
    let e = err_of(rename(&json!({ "from": from, "to": to })));
    assert!(e.contains(&format!("fs.rename({from} -> {to})")), "{e}");
}

#[test]
fn rename_out_of_scope_destination_is_refused() {
    let dir = tmp();
    std::fs::write(dir.path().join("a"), "1").unwrap();
    let e = err_of(rename(
        &json!({ "from": p(dir.path(), "a"), "to": out_of_scope("a") }),
    ));
    assert!(e.contains("outside capability roots"), "{e}");
    assert!(dir.path().join("a").exists());
}

#[test]
fn copy_file_and_reject_directory_source() {
    let dir = tmp();
    std::fs::write(dir.path().join("a"), "data").unwrap();
    copy(&json!({ "from": p(dir.path(), "a"), "to": p(dir.path(), "b") })).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b")).unwrap(),
        "data"
    );
    assert!(dir.path().join("a").exists());

    std::fs::create_dir(dir.path().join("d")).unwrap();
    let e = err_of(copy(
        &json!({ "from": p(dir.path(), "d"), "to": p(dir.path(), "e") }),
    ));
    assert!(e.contains("is not a regular file"), "{e}");

    let e = err_of(copy(
        &json!({ "from": p(dir.path(), "a"), "to": p(dir.path(), "no/dir/b") }),
    ));
    assert!(e.starts_with("fs.copy("), "{e}");
}

#[test]
fn copy_dir_copies_nested_tree() {
    let dir = tmp();
    std::fs::create_dir_all(dir.path().join("src/a/b")).unwrap();
    std::fs::write(dir.path().join("src/top.txt"), "t").unwrap();
    std::fs::write(dir.path().join("src/a/b/deep.txt"), "d").unwrap();
    std::fs::create_dir(dir.path().join("src/empty")).unwrap();

    copy_dir(&json!({ "from": p(dir.path(), "src"), "to": p(dir.path(), "dst/nested") })).unwrap();
    let dst = dir.path().join("dst/nested");
    assert_eq!(std::fs::read_to_string(dst.join("top.txt")).unwrap(), "t");
    assert_eq!(
        std::fs::read_to_string(dst.join("a/b/deep.txt")).unwrap(),
        "d"
    );
    assert!(dst.join("empty").is_dir());

    let e = err_of(copy_dir(
        &json!({ "from": p(dir.path(), "missing"), "to": p(dir.path(), "x") }),
    ));
    assert!(e.starts_with("fs.copyDir("), "{e}");
}

#[test]
fn hardlink_shares_content() {
    let dir = tmp();
    std::fs::write(dir.path().join("orig"), "v1").unwrap();
    hardlink(&json!({ "original": p(dir.path(), "orig"), "link": p(dir.path(), "link") })).unwrap();
    std::fs::write(dir.path().join("orig"), "v2").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("link")).unwrap(),
        "v2"
    );

    let e = err_of(hardlink(&json!({
        "original": p(dir.path(), "missing"),
        "link": p(dir.path(), "l2"),
    })));
    assert!(e.starts_with("fs.hardlink:"), "{e}");
}

#[test]
fn symlink_to_target_outside_roots_is_refused_before_creation() {
    let dir = tmp();
    let link = p(dir.path(), "link");
    let e = err_of(symlink(
        &json!({ "target": out_of_scope("t"), "link": link }),
    ));
    assert!(e.contains("outside capability roots"), "{e}");

    // Relative target is resolved against the link parent, so climbing out
    // of every root is refused as well.
    let depth = dir.path().components().count() + 2;
    let rel = format!("{}tynd-out-of-scope-test-dir", "../".repeat(depth));
    let e = err_of(symlink(&json!({ "target": rel, "link": link })));
    assert!(e.contains("outside capability roots"), "{e}");
    assert!(std::fs::symlink_metadata(dir.path().join("link")).is_err());
}

#[cfg(windows)]
#[test]
fn symlink_drive_relative_target_is_refused() {
    let dir = tmp();
    let e = err_of(symlink(
        &json!({ "target": "C:foo", "link": p(dir.path(), "link") }),
    ));
    assert!(e.contains("drive-relative"), "{e}");
}

#[test]
fn trash_out_of_scope_is_refused() {
    let e = err_of(trash_path(&json!({ "path": out_of_scope("x") })));
    assert!(e.contains("outside capability roots"), "{e}");
}

#[test]
fn fs_path_to_utf8_accepts_utf8() {
    let ok = fs_path_to_utf8(PathBuf::from("a/b"), "ctx").unwrap();
    assert_eq!(ok.as_str(), "a/b");
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::symlink as os_symlink;

    use super::*;

    #[test]
    fn relative_symlink_round_trips_through_readlink() {
        let dir = tmp();
        std::fs::write(dir.path().join("target.txt"), "via link").unwrap();
        symlink(&json!({ "target": "target.txt", "link": p(dir.path(), "link") })).unwrap();
        assert_eq!(
            readlink(&json!({ "path": p(dir.path(), "link") })).unwrap(),
            json!("target.txt"),
            "literal target is stored"
        );
        assert_eq!(
            read_text(&json!({ "path": p(dir.path(), "link") })).unwrap(),
            json!("via link")
        );
    }

    #[test]
    fn dangling_symlink_inside_root_is_allowed() {
        let dir = tmp();
        symlink(&json!({ "target": "later.txt", "link": p(dir.path(), "link") })).unwrap();
        assert!(std::fs::symlink_metadata(dir.path().join("link")).is_ok());
        // Operations through the dangling link are rejected, not mistaken
        // for creatable destinations.
        let e = err_of(write_text(
            &json!({ "path": p(dir.path(), "link"), "content": "x" }),
        ));
        assert!(e.contains("dangling"), "{e}");
    }

    #[test]
    fn symlink_escaping_roots_is_refused_on_access() {
        let dir = tmp();
        os_symlink("/", dir.path().join("root")).unwrap();
        let e = err_of(read_dir(&json!({ "path": p(dir.path(), "root") })));
        assert!(e.contains("escapes capability roots"), "{e}");
        assert_eq!(
            exists(&json!({ "path": p(dir.path(), "root") })).unwrap(),
            json!(false)
        );
    }

    #[test]
    fn remove_deletes_link_not_target() {
        let dir = tmp();
        std::fs::create_dir(dir.path().join("real")).unwrap();
        std::fs::write(dir.path().join("real/keep"), "k").unwrap();
        os_symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
        remove(&json!({ "path": p(dir.path(), "link"), "recursive": true })).unwrap();
        assert!(std::fs::symlink_metadata(dir.path().join("link")).is_err());
        assert!(dir.path().join("real/keep").exists());
    }

    #[test]
    fn stat_follows_symlink_and_copy_dir_skips_links() {
        let dir = tmp();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/f"), "x").unwrap();
        os_symlink("f", dir.path().join("src/l")).unwrap();
        copy_dir(&json!({ "from": p(dir.path(), "src"), "to": p(dir.path(), "dst") })).unwrap();
        assert!(dir.path().join("dst/f").exists());
        assert!(std::fs::symlink_metadata(dir.path().join("dst/l")).is_err());
    }

    #[test]
    fn non_utf8_dir_entry_is_reported_not_mangled() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt as _};
        let dir = tmp();
        let bad = OsStr::from_bytes(b"bad\xff");
        if std::fs::write(dir.path().join(bad), "").is_err() {
            return; // FS (e.g. APFS) refuses non-UTF-8 names.
        }
        let e = err_of(read_dir(&json!({ "path": dir.path().to_str().unwrap() })));
        assert!(e.contains("not valid UTF-8"), "{e}");
    }
}
