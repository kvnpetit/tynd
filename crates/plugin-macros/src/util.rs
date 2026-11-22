//! Crate-internal string-case helpers shared by `analyze` and
//! `plugin_impl`. Lifted here so the codegen stays byte-for-byte
//! consistent across both routes.

/// PascalCase → camelCase: lowercases the first ASCII char, leaves the rest unchanged.
pub(crate) fn pascal_to_camel(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().chain(chars).collect(),
    }
}

/// `open_db` → `OpenDb`, `open` → `Open`. Stops at `_`, uppercases the
/// next char.
pub(crate) fn snake_to_pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut up = true;
    for ch in s.chars() {
        if ch == '_' {
            up = true;
            continue;
        }
        if up {
            out.extend(ch.to_uppercase());
            up = false;
        } else {
            out.push(ch);
        }
    }
    out
}
