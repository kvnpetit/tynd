//! Parse the `#[tynd::handler(api = "<name>")]` arguments and walk the
//! `impl` block to classify methods.
//!
//! Mirrors the `analyze`/`codegen` split used by the plugin-sdk macro
//! (`tynd_plugin_macros`) — the analysis pass collects everything the
//! generation step needs into a small `Plan`, so codegen never has to
//! re-walk the AST.

use proc_macro2::{Span, TokenStream};
use syn::{
    spanned::Spanned, Attribute, Expr, ExprLit, FnArg, ImplItem, ImplItemFn, Lit, Meta, Pat,
    PatType, Receiver, ReturnType, Type,
};

/// Names of the `path` segments accepted as our `skip` marker —
/// `tynd::skip`, `tynd_host_macros::skip`, and the bare `skip` form
/// (for users who `use tynd_host_macros::skip;` at the top of the
/// module).
const SKIP_PATH_TAILS: &[&[&str]] = &[
    &["tynd", "skip"],
    &["tynd_host_macros", "skip"],
    &["skip"],
];

/// Descriptor for one wire-exposed handler method.
#[derive(Debug)]
pub(crate) struct Handler {
    /// Rust ident — used to invoke the method on `Self` from the
    /// generated dispatch arm.
    pub(crate) ident: syn::Ident,
    /// camelCase wire name — used as the `match` arm literal in
    /// dispatch (`read_text` → `"readText"`).
    pub(crate) wire_name: String,
    /// True when the method declares `&self` / `self` — the dispatch
    /// arm must construct a temporary `Self` (unit struct) before
    /// invoking. False when the method has no receiver — invoke as
    /// `Self::method(...)` directly.
    pub(crate) has_receiver: bool,
    /// Per-argument descriptors, in declaration order, excluding any
    /// receiver.
    pub(crate) args: Vec<HandlerArg>,
    /// Return type classification — drives the per-arm response wiring
    /// (`Result<T, E>` vs plain `T` vs `()`).
    pub(crate) ret: HandlerReturn,
    /// Concatenated `///` doc comment lines, one per attribute. Used
    /// by the sibling `__tynd_handler_doc` fn for future gen-types
    /// JSDoc forwarding.
    pub(crate) docs: Vec<String>,
}

/// Per-argument descriptor.
#[derive(Debug)]
pub(crate) struct HandlerArg {
    /// Original Rust ident — used as the temp-binding name in the
    /// generated dispatch arm.
    pub(crate) ident: syn::Ident,
    /// camelCase wire key — the JSON object field looked up in `args`.
    pub(crate) wire_key: String,
    /// The argument type, verbatim. Drives `serde_json::from_value::<T>`.
    pub(crate) ty: Type,
    /// True when the type is syntactically `Option<…>` — the dispatch
    /// arm tolerates the field being absent, defaulting to `None`.
    pub(crate) optional: bool,
}

/// Return-type classification.
#[derive(Debug)]
pub(crate) enum HandlerReturn {
    /// `-> ()` (no return arrow) — emit `Value::Null`.
    Unit,
    /// `-> Result<T, E>` — `T` to_value'd on Ok, `E.into()` on Err.
    Result,
    /// `-> T` (any other shape) — emit `to_value(t)` directly.
    Plain,
}

/// Plan produced by [`analyze_impl`] and consumed by `codegen`.
#[derive(Debug)]
pub(crate) struct Plan {
    pub(crate) handlers: Vec<Handler>,
}

/// Parse the macro attribute argument list. Currently the only accepted
/// shape is `api = "<literal>"`; everything else is rejected with a
/// span pointed at the offending tokens.
pub(crate) fn parse_api_arg(args: TokenStream) -> syn::Result<String> {
    if args.is_empty() {
        return Err(syn::Error::new(
            Span::call_site(),
            "#[tynd::handler] requires `api = \"<name>\"` — the api name is used in error \
             messages (`<api>.<method>: ...`) and must match the wire-format identifier",
        ));
    }
    // Parse as a single `Meta::NameValue` — `syn::Meta` round-trips
    // `key = "value"` directly without us hand-rolling a parser.
    let meta: Meta = syn::parse2(args)?;
    let nv = match meta {
        Meta::NameValue(nv) => nv,
        other => {
            return Err(syn::Error::new(
                other.span(),
                "#[tynd::handler] expected `api = \"<name>\"` (single name-value pair)",
            ));
        },
    };
    if !nv.path.is_ident("api") {
        return Err(syn::Error::new(
            nv.path.span(),
            "#[tynd::handler] only accepts `api = \"<name>\"` (got an unexpected key)",
        ));
    }
    match nv.value {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(syn::Error::new(
            other.span(),
            "#[tynd::handler] `api` value must be a string literal",
        )),
    }
}

/// Walk the `impl` block, classify each method, return the dispatch
/// `Plan`. Methods marked `#[tynd::skip]` are silently skipped (and
/// their attribute stripped by the codegen pass). Non-fn items inside
/// the `impl` are passed through untouched.
pub(crate) fn analyze_impl(items: &[ImplItem]) -> syn::Result<Plan> {
    let mut handlers = Vec::new();
    for item in items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        if has_skip(&method.attrs) {
            continue;
        }
        if !is_pub(&method.vis) {
            // Private helpers stay invisible to dispatch — same rule
            // as `pub mod commands { ... }` in the plugin-sdk macro.
            continue;
        }
        handlers.push(classify_method(method)?);
    }
    Ok(Plan { handlers })
}

fn classify_method(method: &ImplItemFn) -> syn::Result<Handler> {
    let ident = method.sig.ident.clone();
    let wire_name = to_camel_case(&ident.to_string());

    let mut has_receiver = false;
    let mut args = Vec::new();
    for input in &method.sig.inputs {
        match input {
            FnArg::Receiver(Receiver {
                mutability,
                reference,
                ..
            }) => {
                if mutability.is_some() && reference.is_some() {
                    return Err(syn::Error::new(
                        input.span(),
                        "#[tynd::handler] cannot wrap a `&mut self` method — dispatch is \
                         called from a borrowed JSON value with no caller-side handle to a \
                         mutable instance. Park state inside a `LazyLock<Mutex<…>>` and \
                         take `&self` (or no receiver) instead.",
                    ));
                }
                has_receiver = true;
            },
            FnArg::Typed(PatType { pat, ty, .. }) => {
                let Pat::Ident(pat_ident) = pat.as_ref() else {
                    return Err(syn::Error::new(
                        pat.span(),
                        "#[tynd::handler] only supports plain identifier parameters \
                         (`name: T`) — destructuring patterns aren't expressible in the \
                         generated `serde_json::from_value` call.",
                    ));
                };
                let arg_ident = pat_ident.ident.clone();
                let wire_key = to_camel_case(&arg_ident.to_string());
                let optional = is_option_type(ty);
                args.push(HandlerArg {
                    ident: arg_ident,
                    wire_key,
                    ty: (**ty).clone(),
                    optional,
                });
            },
        }
    }

    let ret = classify_return(&method.sig.output);
    let docs = extract_doc_lines(&method.attrs);

    Ok(Handler {
        ident,
        wire_name,
        has_receiver,
        args,
        ret,
        docs,
    })
}

fn classify_return(out: &ReturnType) -> HandlerReturn {
    match out {
        ReturnType::Default => HandlerReturn::Unit,
        ReturnType::Type(_, ty) => {
            if is_unit_type(ty) {
                HandlerReturn::Unit
            } else if is_result_type(ty) {
                HandlerReturn::Result
            } else {
                HandlerReturn::Plain
            }
        },
    }
}

fn is_unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

fn is_result_type(ty: &Type) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    tp.path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "Result")
}

fn is_option_type(ty: &Type) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };
    tp.path
        .segments
        .last()
        .is_some_and(|seg| seg.ident == "Option")
}

fn is_pub(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

/// True when an attribute matches one of the recognised `skip` paths.
/// Same matching rules as the plugin-sdk macro — accept fully-qualified
/// paths *or* the bare ident after a `use`.
pub(crate) fn has_skip(attrs: &[Attribute]) -> bool {
    attrs.iter().any(is_skip_attr)
}

pub(crate) fn is_skip_attr(attr: &Attribute) -> bool {
    let segs: Vec<String> = attr
        .path()
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    SKIP_PATH_TAILS.iter().any(|cand| {
        cand.len() == segs.len()
            && cand
                .iter()
                .zip(segs.iter())
                .all(|(a, b)| *a == b.as_str())
    })
}

/// Pull the `///` doc-comment text out of an attribute list. Each line
/// keeps its leading-space-stripped body so the assembled blob looks
/// like the source comment when re-emitted into JSDoc.
pub(crate) fn extract_doc_lines(attrs: &[Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        let Meta::NameValue(nv) = &attr.meta else {
            continue;
        };
        if let Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) = &nv.value
        {
            // Match rustdoc's own convention: strip ONE leading space
            // (the conventional `/// foo` → `" foo"`) so JSDoc consumers
            // don't have to re-trim.
            let raw = s.value();
            let trimmed = raw.strip_prefix(' ').unwrap_or(&raw).to_string();
            out.push(trimmed);
        }
    }
    out
}

/// Convert a Rust `snake_case` ident to `camelCase` for the wire name.
/// Leaves already-camelCase idents untouched (no `_` → no change).
/// Preserves leading underscores by skipping them — `_internal` would
/// classify as `Internal` here, but the macro never sees private fns
/// so the edge case doesn't matter in practice.
fn to_camel_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut upper_next = false;
    for c in s.chars() {
        if c == '_' {
            upper_next = true;
            continue;
        }
        if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_to_camel_basic() {
        assert_eq!(to_camel_case("read_text"), "readText");
        assert_eq!(to_camel_case("read"), "read");
        assert_eq!(to_camel_case("write_html"), "writeHtml");
        assert_eq!(to_camel_case("alreadyCamel"), "alreadyCamel");
        assert_eq!(to_camel_case("a_b_c"), "aBC");
    }
}
