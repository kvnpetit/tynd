//! Pure-inspection pass. Output is a [`Plan`] consumed by [`crate::codegen`].

use syn::{Attribute, Item, ItemMod, Path, PathArguments, Type, TypePath};

use crate::util::pascal_to_camel;

/// Supported `skip` attribute paths.
const SKIP_PATH_TAILS: &[&[&str]] = &[
    &["tynd_plugin", "skip"],
    &["tynd_plugin_macros", "skip"],
    &["skip"],
];

/// Supported `plugin_command` attribute paths.
const PLUGIN_COMMAND_PATH_TAILS: &[&[&str]] = &[
    &["tynd_plugin", "plugin_command"],
    &["tynd_plugin_macros", "plugin_command"],
    &["plugin_command"],
];

/// Plan produced by [`analyze_module`] and consumed by `codegen`.
#[derive(Debug)]
pub(crate) struct Plan {
    /// Identifier of the analyzed module.
    pub(crate) mod_ident: syn::Ident,
    /// Every exported-visible `struct` / `enum` / `type` not annotated with `#[skip]`.
    pub(crate) exported_idents: Vec<syn::Ident>,
    /// Type aliases emitted manually because specta does not walk aliases.
    pub(crate) type_aliases: Vec<TypeAlias>,
    /// Convention-detected `<Stem>Args` / `<Stem>Result` pairs.
    pub(crate) commands: Vec<CommandPair>,
}

#[derive(Debug, Clone)]
/// Captured `pub type X = ...` declaration.
pub(crate) struct TypeAlias {
    pub(crate) ident: syn::Ident,
    /// TypeScript rendering of the right-hand side.
    pub(crate) ts_target: String,
}

#[derive(Debug, Clone)]
/// One `_commands` catalog entry.
pub(crate) struct CommandPair {
    /// Typed-client method name.
    pub(crate) method: String,
    /// Args struct ident.
    pub(crate) args_ident: syn::Ident,
    /// Result struct ident when present.
    pub(crate) result_ident: Option<syn::Ident>,
}

/// True when an attribute matches one of the recognised `skip` paths.
pub(crate) fn is_skip_attr(attr: &Attribute) -> bool {
    matches_known_path(attr.path(), SKIP_PATH_TAILS)
}

/// True when an attribute matches one of the recognised
/// `plugin_command` paths.
pub(crate) fn is_plugin_command_attr(attr: &Attribute) -> bool {
    matches_known_path(attr.path(), PLUGIN_COMMAND_PATH_TAILS)
}

#[derive(Debug, Default, Clone, Copy)]
/// Parsed `#[plugin_command(...)]` flags.
pub(crate) struct PluginCommandFlags {
    /// Whether to add rkyv derives.
    pub(crate) zero_copy: bool,
}

/// Parse `#[plugin_command(...)]` flags. Unknown idents become a
/// `syn::Error` so the caller can surface them at the invocation site.
pub(crate) fn parse_plugin_command_flags(attrs: &[Attribute]) -> syn::Result<PluginCommandFlags> {
    let mut out = PluginCommandFlags::default();
    for attr in attrs {
        if !is_plugin_command_attr(attr) {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("zero_copy") {
                out.zero_copy = true;
                Ok(())
            } else {
                let ident = meta
                    .path
                    .get_ident()
                    .map_or_else(|| "<non-ident>".to_string(), syn::Ident::to_string);
                Err(meta.error(format!(
                    "unknown #[plugin_command(...)] flag `{ident}` (recognised: `zero_copy`)"
                )))
            }
        })?;
    }
    Ok(out)
}

/// Strip every `#[plugin_command(...)]` attribute so rustc never sees
/// the marker — the macro consumes it on the way out.
pub(crate) fn strip_plugin_command_attrs(attrs: &mut Vec<Attribute>) {
    attrs.retain(|attr| !is_plugin_command_attr(attr));
}

/// True when this `Path` matches any of the candidate tail-segment
/// lists. Compares on segments only so the user can spell the attribute
/// in any of the supported forms.
fn matches_known_path(path: &Path, candidates: &[&[&str]]) -> bool {
    let segs: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    candidates.iter().any(|cand| {
        cand.len() == segs.len() && cand.iter().zip(segs.iter()).all(|(a, b)| *a == b.as_str())
    })
}

/// Walk the module body, classify items, return the `Plan`.
pub(crate) fn analyze_module(module: &ItemMod) -> Plan {
    let mut exported_idents = Vec::new();
    let mut type_aliases = Vec::new();
    if let Some((_, items)) = module.content.as_ref() {
        for item in items {
            if let Some(ident) = exportable_ident(item) {
                exported_idents.push(ident.clone());
            }
            if let Item::Type(t) = item {
                if is_export_visible(&t.vis) && !has_skip(&t.attrs) {
                    type_aliases.push(TypeAlias {
                        ident: t.ident.clone(),
                        ts_target: render_ts_type(&t.ty),
                    });
                }
            }
        }
    }
    let commands = detect_command_pairs(&exported_idents);
    Plan {
        mod_ident: module.ident.clone(),
        exported_idents,
        type_aliases,
        commands,
    }
}

/// Best-effort Rust type to TypeScript renderer for aliases.
pub(crate) fn render_ts_type(ty: &Type) -> String {
    let Some(ident) = ty_last_ident(ty) else {
        return "unknown".to_string();
    };
    let name = ident.to_string();
    match name.as_str() {
        "Option" => match first_generic(ty) {
            Some(inner) => format!("{} | null", render_ts_type(inner)),
            None => "unknown".to_string(),
        },
        "Vec" | "HashSet" | "BTreeSet" => match first_generic(ty) {
            Some(inner) => format!("{}[]", render_ts_type(inner)),
            None => "unknown[]".to_string(),
        },
        "HashMap" | "BTreeMap" => {
            let (k, v) = nth_two_generics(ty);
            let k_ts = k.map_or_else(|| "string".to_string(), render_ts_type);
            let v_ts = v.map_or_else(|| "unknown".to_string(), render_ts_type);
            format!("{{ [key in {k_ts}]: {v_ts} }}")
        },
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" | "i8" | "i16" | "i32" | "i64" | "i128"
        | "isize" | "f32" | "f64" => "number".to_string(),
        "String" | "str" => "string".to_string(),
        "bool" => "boolean".to_string(),
        _ => name,
    }
}

/// Pair `<Stem>Args` with an optional `<Stem>Result`.
fn detect_command_pairs(idents: &[syn::Ident]) -> Vec<CommandPair> {
    let names: Vec<String> = idents.iter().map(syn::Ident::to_string).collect();
    let has = |needle: &str| names.iter().any(|n| n == needle);

    let mut commands = Vec::new();
    for (idx, name) in names.iter().enumerate() {
        let Some(stem) = name.strip_suffix("Args") else {
            continue;
        };
        if stem.is_empty() {
            continue;
        }
        let method = pascal_to_camel(stem);
        let result_name = format!("{stem}Result");
        let result_ident = if has(&result_name) {
            idents.iter().find(|i| i == &&result_name).cloned()
        } else {
            None
        };
        commands.push(CommandPair {
            method,
            args_ident: idents[idx].clone(),
            result_ident,
        });
    }
    commands
}

/// Return the visible item ident unless it is skipped.
fn exportable_ident(item: &Item) -> Option<&syn::Ident> {
    match item {
        Item::Struct(s) => {
            if !is_export_visible(&s.vis) || has_skip(&s.attrs) {
                None
            } else {
                Some(&s.ident)
            }
        },
        Item::Enum(e) => {
            if !is_export_visible(&e.vis) || has_skip(&e.attrs) {
                None
            } else {
                Some(&e.ident)
            }
        },
        Item::Type(t) => {
            if !is_export_visible(&t.vis) || has_skip(&t.attrs) {
                None
            } else {
                Some(&t.ident)
            }
        },
        _ => None,
    }
}

/// Non-private visibility is reachable by the sibling export function.
fn is_export_visible(vis: &syn::Visibility) -> bool {
    matches!(
        vis,
        syn::Visibility::Public(_) | syn::Visibility::Restricted(_)
    )
}

/// Whether an item has `#[tynd_plugin::skip]`.
pub(crate) fn has_skip(attrs: &[Attribute]) -> bool {
    attrs.iter().any(is_skip_attr)
}

/// Whether the attribute set already carries `#[specta(type = ...)]`,
/// so we don't double-annotate user-customised fields. Unknown specta
/// metas are tolerated — specta grows new flags independently.
pub(crate) fn has_specta_type_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("specta") {
            return false;
        }
        let mut found = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("type") {
                found = true;
            }
            Ok(())
        });
        found
    })
}

/// Decide whether a field type involves a 64-bit integer in a position
/// where TS `number` (`f64`) is the right wire shape. Recognises bare
/// `u64` / `i64` / `usize` / etc. plus `Option<…>` and `Vec<…>` of the
/// same. Returns the wrapper-aware specta replacement.
pub(crate) fn auto_number_replacement(ty: &Type) -> Option<proc_macro2::TokenStream> {
    use quote::quote;
    let ident = ty_last_ident(ty)?;
    let name = ident.to_string();
    match name.as_str() {
        "u64" | "i64" | "u128" | "i128" | "usize" | "isize" => Some(quote!(f64)),
        "Option" => {
            let inner = first_generic(ty)?;
            let inner_repl = auto_number_replacement(inner)?;
            Some(quote!(Option<#inner_repl>))
        },
        "Vec" => {
            let inner = first_generic(ty)?;
            let inner_repl = auto_number_replacement(inner)?;
            Some(quote!(Vec<#inner_repl>))
        },
        _ => None,
    }
}

/// Last segment ident of a `Type::Path` — `Vec<u64>` → `Vec`. Returns
/// `None` for non-path types (refs, tuples, fn pointers).
fn ty_last_ident(ty: &Type) -> Option<&syn::Ident> {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        path.segments.last().map(|seg| &seg.ident)
    } else {
        None
    }
}

/// Return the first generic argument of a path type — `Option<T>` → `T`.
fn first_generic(ty: &Type) -> Option<&Type> {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        let last = path.segments.last()?;
        if let PathArguments::AngleBracketed(args) = &last.arguments {
            for arg in &args.args {
                if let syn::GenericArgument::Type(t) = arg {
                    return Some(t);
                }
            }
        }
    }
    None
}

/// Return the first two generic args of a path type — `HashMap<K, V>` → `(K, V)`.
fn nth_two_generics(ty: &Type) -> (Option<&Type>, Option<&Type>) {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        if let Some(last) = path.segments.last() {
            if let PathArguments::AngleBracketed(args) = &last.arguments {
                let mut iter = args.args.iter().filter_map(|a| match a {
                    syn::GenericArgument::Type(t) => Some(t),
                    _ => None,
                });
                let k = iter.next();
                let v = iter.next();
                return (k, v);
            }
        }
    }
    (None, None)
}

/// Report which canonical derives are still missing so the macro can
/// splice them without producing a rustc duplicate-derive error. The
/// `rkyv` path check disambiguates `serde::Serialize` from
/// `rkyv::Serialize`. Unknown derives pass through untouched.
pub(crate) fn missing_derives(attrs: &[Attribute]) -> MissingDerives {
    let mut serialize = true;
    let mut deserialize = true;
    let mut type_ = true;
    let mut rkyv_archive = true;
    let mut rkyv_serialize = true;
    let mut rkyv_deserialize = true;
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            let segs: Vec<String> = meta
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let last = segs.last().map(String::as_str);
            let is_rkyv_path = segs.iter().any(|s| s == "rkyv");
            match (is_rkyv_path, last) {
                (true, Some("Archive")) => rkyv_archive = false,
                (true, Some("Serialize")) => rkyv_serialize = false,
                (true, Some("Deserialize")) => rkyv_deserialize = false,
                (false, Some("Serialize")) => serialize = false,
                (false, Some("Deserialize")) => deserialize = false,
                (_, Some("Type")) => type_ = false,
                _ => {},
            }
            Ok(())
        });
    }
    MissingDerives {
        core: MissingCoreDerives {
            serialize,
            deserialize,
            type_,
        },
        rkyv: MissingRkyvDerives {
            archive: rkyv_archive,
            serialize: rkyv_serialize,
            deserialize: rkyv_deserialize,
        },
    }
}

#[derive(Debug, Clone, Copy)]
struct MissingCoreDerives {
    serialize: bool,
    deserialize: bool,
    type_: bool,
}

#[derive(Debug, Clone, Copy)]
struct MissingRkyvDerives {
    archive: bool,
    serialize: bool,
    deserialize: bool,
}

/// Which canonical plugin-side derives are missing. The rkyv trio only
/// fires when the item carries `#[plugin_command(zero_copy)]`; serde +
/// specta fire unconditionally.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MissingDerives {
    core: MissingCoreDerives,
    rkyv: MissingRkyvDerives,
}

impl MissingDerives {
    pub(crate) fn needs_specta_crate_attr(self) -> bool {
        self.core.type_
    }

    /// `true` when a serde derive is injected: it then needs
    /// `#[serde(crate = "::tynd_plugin_sdk::serde")]`, otherwise the
    /// derive output does `extern crate serde;`, which fails in plugins
    /// that only depend on `tynd-plugin-sdk` (E0463).
    pub(crate) fn needs_serde_crate_attr(self) -> bool {
        self.core.serialize || self.core.deserialize
    }

    /// Render the missing derives as a token stream, ready to splice
    /// into a `#[derive(...)]` attribute. Returns `None` when nothing
    /// is missing.
    ///
    /// Resolves serde / specta / rkyv via the SDK re-exports
    /// (`::tynd_plugin_sdk::serde::…`) so plugin authors don't need to
    /// version-pin the underlying crates themselves — the SDK is the
    /// single source of truth for the wire-protocol stack.
    pub(crate) fn to_tokens(self, include_rkyv: bool) -> Option<proc_macro2::TokenStream> {
        use quote::quote;
        let mut parts = Vec::new();
        if self.core.serialize {
            parts.push(quote!(::tynd_plugin_sdk::serde::Serialize));
        }
        if self.core.deserialize {
            parts.push(quote!(::tynd_plugin_sdk::serde::Deserialize));
        }
        if self.core.type_ {
            parts.push(quote!(::tynd_plugin_sdk::specta::Type));
        }
        if include_rkyv {
            if self.rkyv.archive {
                parts.push(quote!(::tynd_plugin_sdk::rkyv::Archive));
            }
            if self.rkyv.serialize {
                parts.push(quote!(::tynd_plugin_sdk::rkyv::Serialize));
            }
            if self.rkyv.deserialize {
                parts.push(quote!(::tynd_plugin_sdk::rkyv::Deserialize));
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(quote!(#[derive( #(#parts),* )]))
        }
    }
}
