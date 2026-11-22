use std::path::{Path, PathBuf};

use crate::fsx::absolute;

/// Cargo package and binary name for an app directory name: lowercase ASCII
/// alphanumerics, `-` and `_`, prefixed with `app-` when cargo or the OS would
/// reject it (leading digit, Rust keyword, cargo-reserved or Windows device
/// name). The host build and the binary lookup both derive the name here.
pub(crate) fn sanitize_crate_name(name: &str) -> String {
    let name = sanitize_name_characters(name);
    if needs_app_prefix(&name) {
        format!("app-{name}")
    } else {
        name
    }
}

/// Names cargo refuses for a package or binary, or that collide with a Rust
/// keyword, a built-in crate or a cargo output directory.
const RESERVED_CRATE_NAMES: &[&str] = &[
    // Cargo output directories and built-in crates.
    "build",
    "deps",
    "examples",
    "incremental",
    "test",
    "std",
    "core",
    "alloc",
    "proc_macro",
    "proc-macro",
    // Rust keywords (strict and reserved).
    "abstract",
    "as",
    "async",
    "await",
    "become",
    "box",
    "break",
    "const",
    "continue",
    "crate",
    "do",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "final",
    "fn",
    "for",
    "gen",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "macro",
    "match",
    "mod",
    "move",
    "mut",
    "override",
    "priv",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "try",
    "type",
    "typeof",
    "union",
    "unsafe",
    "unsized",
    "use",
    "virtual",
    "where",
    "while",
    "yield",
    // Windows device names.
    "con",
    "prn",
    "aux",
    "nul",
];

fn needs_app_prefix(name: &str) -> bool {
    if name.starts_with(|character: char| character.is_ascii_digit())
        || RESERVED_CRATE_NAMES.contains(&name)
    {
        return true;
    }
    // Windows device names `com1`-`com9` and `lpt1`-`lpt9`.
    ["com", "lpt"].iter().any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|rest| {
            rest.len() == 1 && rest.as_bytes()[0].is_ascii_digit() && rest != "0"
        })
    })
}

fn sanitize_name_characters(name: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in name.to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() || character == '-' {
            output.push(character);
            separator = false;
        } else if !separator && !output.is_empty() {
            output.push('_');
            separator = true;
        }
    }
    let output = output
        .trim_matches(|character| character == '_' || character == '-')
        .to_owned();
    if output.is_empty() {
        "app".to_owned()
    } else {
        output
    }
}

pub(super) fn walk_up(cwd: &Path) -> impl Iterator<Item = PathBuf> {
    let mut current = absolute(cwd).ok();
    std::iter::from_fn(move || {
        let directory = current.take()?;
        current = directory.parent().map(Path::to_path_buf);
        Some(directory)
    })
}

pub(super) fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|metadata| metadata.is_file())
}

#[cfg(test)]
mod tests {
    use super::sanitize_crate_name;

    #[test]
    fn keeps_valid_names() {
        assert_eq!(sanitize_crate_name("My App"), "my_app");
        assert_eq!(sanitize_crate_name("hello-world"), "hello-world");
        assert_eq!(sanitize_crate_name("app2"), "app2");
        assert_eq!(sanitize_crate_name("console"), "console");
        assert_eq!(sanitize_crate_name("com10"), "com10");
        assert_eq!(sanitize_crate_name("builder"), "builder");
        assert_eq!(sanitize_crate_name("!!!"), "app");
    }

    #[test]
    fn prefixes_names_starting_with_a_digit() {
        assert_eq!(sanitize_crate_name("2048"), "app-2048");
        assert_eq!(sanitize_crate_name("3d viewer"), "app-3d_viewer");
    }

    #[test]
    fn prefixes_reserved_names() {
        for name in [
            "build",
            "deps",
            "examples",
            "incremental",
            "test",
            "self",
            "crate",
            "super",
            "fn",
            "match",
            "std",
            "con",
            "NUL",
            "aux",
            "prn",
            "com1",
            "COM9",
            "lpt1",
            "lpt9",
        ] {
            let sanitized = sanitize_crate_name(name);
            assert_eq!(
                sanitized,
                format!("app-{}", name.to_ascii_lowercase()),
                "{name}"
            );
        }
    }
}
