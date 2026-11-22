use super::*;
use crate::{PolicyInput, ScopeInput};

/// Absolute spelling of a `/`-separated path on the current OS.
fn abs(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_string()
    }
}

/// Fs globs use `/` on every OS.
fn abs_glob(pattern: &str) -> String {
    if cfg!(windows) {
        format!("C:{pattern}")
    } else {
        pattern.to_string()
    }
}

fn dir(path: &str, recursive: bool, write: bool) -> PathGrant {
    PathGrant {
        path: abs(path),
        directory: true,
        recursive,
        write,
    }
}

fn file(path: &str, write: bool) -> PathGrant {
    PathGrant {
        path: abs(path),
        directory: false,
        recursive: false,
        write,
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(ToString::to_string).collect()
}

fn policy(
    fs_allow: &[&str],
    fs_deny: &[&str],
    http_allow: &[&str],
    http_deny: &[&str],
) -> SecurityPolicy {
    let globs = |items: &[&str]| items.iter().map(|item| abs_glob(item)).collect::<Vec<_>>();
    SecurityPolicy::from_input(
        PolicyInput {
            fs: ScopeInput {
                allow: Vec::new(),
                deny: globs(fs_deny),
            },
            http: ScopeInput {
                allow: Vec::new(),
                deny: strings(http_deny),
            },
            ..PolicyInput::default()
        },
        globs(fs_allow),
        strings(http_allow),
    )
    .expect("valid policy")
}

#[test]
fn file_grant_covers_only_that_file() {
    let mut grants = Grants::default();
    grants.add_path(file("/data/a.txt", false)).expect("grant");
    assert!(grants.allows_path(&abs("/data/a.txt"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/data/a.txt.bak"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/data/a.txt/child"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/data/b.txt"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/data"), FsAccess::Read));
}

#[test]
fn read_only_grant_never_authorizes_a_write() {
    let mut grants = Grants::default();
    grants.add_path(dir("/data", true, false)).expect("grant");
    assert!(grants.allows_path(&abs("/data/x/y"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/data/x/y"), FsAccess::Write));
    assert!(!grants.allows_path(&abs("/data"), FsAccess::Write));
}

#[test]
fn directory_grant_depth_follows_recursive() {
    let mut grants = Grants::default();
    grants.add_path(dir("/flat", false, true)).expect("grant");
    grants.add_path(dir("/deep", true, true)).expect("grant");
    assert!(grants.allows_path(&abs("/flat"), FsAccess::Write));
    assert!(grants.allows_path(&abs("/flat/a"), FsAccess::Write));
    assert!(!grants.allows_path(&abs("/flat/a/b"), FsAccess::Read));
    assert!(grants.allows_path(&abs("/deep/a/b/c"), FsAccess::Write));
    // Component boundary: a sibling sharing the prefix is not covered.
    assert!(!grants.allows_path(&abs("/deeper/a"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/flatter"), FsAccess::Read));
}

#[test]
fn traversal_spellings_cannot_escape_a_grant() {
    let mut grants = Grants::default();
    grants.add_path(dir("/data", true, true)).expect("grant");
    assert!(!grants.allows_path(&abs("/data/../etc/passwd"), FsAccess::Read));
    assert!(!grants.allows_path(&abs("/data/./../secret"), FsAccess::Read));
}

#[cfg(windows)]
#[test]
fn windows_grants_match_case_insensitively() {
    let mut grants = Grants::default();
    grants.add_path(dir("/Data", true, false)).expect("grant");
    assert!(grants.allows_path("c:\\DATA\\Sub\\f.txt", FsAccess::Read));
    assert!(grants.allows_path("C:/data/sub/f.txt", FsAccess::Read));
}

#[test]
fn invalid_path_grants_are_rejected() {
    let mut grants = Grants::default();
    let relative = PathGrant {
        path: "relative/path".into(),
        directory: true,
        recursive: true,
        write: false,
    };
    assert!(grants.add_path(relative).is_err());
    let empty = PathGrant {
        path: String::new(),
        ..file("/x", false)
    };
    assert!(grants.add_path(empty).is_err());
    let control = PathGrant {
        path: abs("/a\u{0}b"),
        ..file("/x", false)
    };
    assert!(grants.add_path(control).is_err());
    // `recursive` is meaningless (and suspicious) on a file.
    let recursive_file = PathGrant {
        recursive: true,
        ..file("/a.txt", false)
    };
    assert!(grants.add_path(recursive_file).is_err());
    assert!(grants.is_empty());
}

#[test]
fn regrant_replaces_and_revoke_removes() {
    let mut grants = Grants::default();
    grants.add_path(dir("/data", true, true)).expect("grant");
    grants
        .add_path(dir("/data", false, false))
        .expect("regrant");
    assert_eq!(grants.snapshot().paths.len(), 1);
    assert!(!grants.allows_path(&abs("/data/a"), FsAccess::Write));
    assert!(!grants.allows_path(&abs("/data/a/b"), FsAccess::Read));
    assert!(grants.revoke_path(&abs("/data")));
    assert!(!grants.revoke_path(&abs("/data")));
    assert!(!grants.allows_path(&abs("/data/a"), FsAccess::Read));
    assert!(grants.is_empty());
}

#[test]
fn path_grants_are_capped() {
    let mut grants = Grants::default();
    for index in 0..MAX_PATH_GRANTS {
        grants
            .add_path(file(&format!("/f{index}"), false))
            .expect("under the cap");
    }
    assert!(grants.add_path(file("/one-more", false)).is_err());
    // Replacing an existing grant is still possible at the cap.
    grants.add_path(file("/f0", true)).expect("replace");
}

#[test]
fn url_patterns_must_name_an_http_host() {
    for bad in [
        "**",
        "*",
        "ftp://example.com/**",
        "https://**",
        "https://*/**",
        "https://ex*ample.com/**",
        "https://user@example.com/**",
        "https:///path",
        "https://example.com/\u{7}",
    ] {
        assert!(validate_url_pattern(bad).is_err(), "{bad} must be rejected");
    }
    for good in [
        "https://example.com/**",
        "https://*.example.com/**",
        "http://localhost:8080/api/*",
    ] {
        validate_url_pattern(good).unwrap_or_else(|error| panic!("{good}: {error}"));
    }
}

#[test]
fn url_grants_match_and_revoke() {
    let mut grants = Grants::default();
    grants
        .add_url("https://api.example.com/v1/**")
        .expect("grant");
    assert!(grants.allows_url("https://api.example.com/v1/users"));
    assert!(grants.allows_url("https://API.example.com:443/v1/users"));
    assert!(!grants.allows_url("https://api.example.com/v2/users"));
    assert!(!grants.allows_url("https://evil.example.com/v1/users"));
    assert!(!grants.allows_url("https://api.example.com.evil.test/v1/users"));
    // Encoded separators and dot segments are never granted.
    assert!(!grants.allows_url("https://api.example.com/v1/..%2Fadmin"));
    assert!(!grants.allows_url("https://api.example.com/v1/%2e%2e/admin"));
    assert!(!grants.allows_url("https://user:pw@api.example.com/v1/x"));
    assert!(grants.revoke_url("https://api.example.com/v1/**"));
    assert!(!grants.revoke_url("https://api.example.com/v1/**"));
    assert!(!grants.allows_url("https://api.example.com/v1/users"));
}

#[test]
fn fs_grants_never_override_deny_rules() {
    let policy = policy(&["/allowed/**"], &["/data/secret/**"], &[], &[]);
    let mut grants = Grants::default();
    grants.add_path(dir("/data", true, true)).expect("grant");
    let check = |path: &str, in_roots| {
        policy.check_fs_with_grants(&abs(path), &grants, FsAccess::Write, in_roots)
    };
    assert!(check("/data/ok.txt", true).is_ok());
    assert!(check("/data/secret/key", true).is_err());
    assert!(check("/data/secret/key", false).is_err());
    assert!(check("/allowed/x", true).is_ok());
    // Neither allowed by the policy nor granted.
    assert!(check("/other/x", true).is_err());
}

#[test]
fn outside_roots_only_a_grant_admits_even_without_a_policy() {
    let unconfigured = SecurityPolicy::default();
    let mut grants = Grants::default();
    grants.add_path(dir("/media", true, false)).expect("grant");
    let read =
        |path: &str| unconfigured.check_fs_with_grants(&abs(path), &grants, FsAccess::Read, false);
    assert!(read("/media/photo.jpg").is_ok());
    assert!(read("/elsewhere/photo.jpg").is_err());
    let error = unconfigured
        .check_fs_with_grants(&abs("/media/photo.jpg"), &grants, FsAccess::Write, false)
        .expect_err("read-only");
    assert!(error.contains("read-only"), "{error}");
    // Inside the roots an unconfigured policy keeps allowing everything.
    assert!(unconfigured
        .check_fs_with_grants(
            &abs("/elsewhere/x"),
            &Grants::default(),
            FsAccess::Write,
            true
        )
        .is_ok());
}

#[test]
fn http_grants_extend_a_configured_allow_list_but_not_its_denies() {
    let policy = policy(
        &[],
        &[],
        &["https://app.example.com/**"],
        &["https://cdn.example.com/private/**"],
    );
    let mut grants = Grants::default();
    grants.add_url("https://cdn.example.com/**").expect("grant");
    assert!(policy
        .check_http_with_grants("https://app.example.com/x", &grants)
        .is_ok());
    assert!(policy
        .check_http_with_grants("https://cdn.example.com/public/a.js", &grants)
        .is_ok());
    assert!(policy
        .check_http_with_grants("https://cdn.example.com/private/a.js", &grants)
        .is_err());
    assert!(policy
        .check_http_with_grants("https://other.example.com/", &grants)
        .is_err());
    assert!(policy
        .check_http_configured_with_grants("https://cdn.example.com/public/a.js", &grants)
        .is_ok());
    assert!(SecurityPolicy::default()
        .check_http_configured_with_grants("https://cdn.example.com/public/a.js", &grants)
        .is_err());
}

#[test]
fn snapshot_serializes_and_rejects_unknown_fields() {
    let mut grants = Grants::default();
    grants.add_path(dir("/data", true, false)).expect("grant");
    grants.add_url("https://example.com/**").expect("grant");
    let json = serde_json::to_string(&grants.snapshot()).expect("serialize");
    assert!(json.contains("\"recursive\":true"), "{json}");
    let parsed: GrantsSnapshot = serde_json::from_str(&json).expect("round trip");
    assert_eq!(parsed, grants.snapshot());
    let unknown: Result<GrantsSnapshot, _> =
        serde_json::from_str(r#"{"paths":[{"path":"/x","everything":true}]}"#);
    assert!(unknown.is_err());
}

#[test]
fn outside_roots_an_explicit_allow_rule_admits_but_an_empty_allow_list_does_not() {
    let grants = Grants::default();
    let allowing = policy(&["/projects/**"], &["/projects/secret/**"], &[], &[]);
    let outside = |policy: &SecurityPolicy, path: &str| {
        policy.check_fs_with_grants(&abs(path), &grants, FsAccess::Write, false)
    };
    assert!(outside(&allowing, "/projects/app/main.ts").is_ok());
    assert!(outside(&allowing, "/projects/secret/key").is_err());
    assert!(outside(&allowing, "/elsewhere/x").is_err());
    // Configured with no allow rule: inside the roots everything is open,
    // outside them nothing is.
    let empty = policy(&[], &[], &[], &[]);
    assert!(outside(&empty, "/projects/app/main.ts").is_err());
    assert!(empty
        .check_fs_with_grants(&abs("/projects/x"), &grants, FsAccess::Write, true)
        .is_ok());
}

#[test]
fn fs_allow_anchor_is_the_wildcard_free_leading_directory() {
    let policy = policy(
        &[
            "/projects/**",
            "/projects/app/src/*.ts",
            "/docs/readme.md",
            "**/*.txt",
        ],
        &[],
        &[],
        &[],
    );
    let anchor = |path: &str| policy.fs_allow_anchor(&abs(path));
    assert_eq!(
        anchor("/projects/app/src/main.ts"),
        Some(abs_glob("/projects/app/src"))
    );
    assert_eq!(
        anchor("/projects/app/other.js"),
        Some(abs_glob("/projects"))
    );
    assert_eq!(anchor("/docs/readme.md"), Some(abs_glob("/docs")));
    // A rule starting with a wildcard has no anchor outside the roots.
    assert_eq!(anchor("/elsewhere/notes.txt"), None);
    assert_eq!(anchor("/nowhere/x"), None);
    assert_eq!(
        SecurityPolicy::default().fs_allow_anchor(&abs("/projects/x")),
        None
    );
}
