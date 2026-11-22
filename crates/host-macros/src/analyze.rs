//! Parse the `#[tynd::handler(api = "<name>")]` arguments and walk the
//! `impl` block to classify methods.
//!
//! Mirrors the `analyze`/`codegen` split used by the plugin-sdk macro
//! (`tynd_plugin_macros`) — the analysis pass collects everything the
//! generation step needs into a small `Plan`, so codegen never has to
//! re-walk the AST.

use proc_macro2::{Span, TokenStream};
use std::collections::HashMap;
use syn::{
    spanned::Spanned, Attribute, Expr, ExprLit, FnArg, ImplItem, ImplItemFn, Lit, Meta, Pat,
    PatType, Receiver, ReceiverKind, ReturnType, Safety, Signature, Type,
};

/// Names of the `path` segments accepted as our `skip` marker —
/// `tynd::skip`, `tynd_host_macros::skip`, and the bare `skip` form
/// (for users who `use tynd_host_macros::skip;` at the top of the
/// module).
const SKIP_PATH_TAILS: &[&[&str]] = &[&["tynd", "skip"], &["tynd_host_macros", "skip"], &["skip"]];

/// Descriptor for one wire-exposed handler method.
#[derive(Debug)]
pub(crate) struct Handler {
    /// Method identifier on the impl block.
    pub(crate) ident: syn::Ident,
    /// camelCase wire name — used as the `match` arm literal in
    /// dispatch (`read_text` → `"readText"`).
    pub(crate) wire_name: String,
    /// True when the method declares `&self` (the only receiver accepted) — the dispatch
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
        }) => {
            let value = s.value();
            let mut chars = value.chars();
            if !chars
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                || !chars.all(|c| c.is_ascii_alphanumeric())
            {
                return Err(syn::Error::new(
                    s.span(),
                    "#[tynd::handler] `api` must start with an ASCII letter and contain only \
                     ASCII letters or digits",
                ));
            }
            Ok(value)
        },
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
    let mut wire_names = HashMap::<String, Span>::new();
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
        let handler = classify_method(method)?;
        if let Some(previous) =
            wire_names.insert(handler.wire_name.clone(), method.sig.ident.span())
        {
            let mut error = syn::Error::new(
                method.sig.ident.span(),
                format!(
                    "duplicate Tynd wire method '{}' after camelCase conversion",
                    handler.wire_name
                ),
            );
            error.combine(syn::Error::new(
                previous,
                "first method with this wire name",
            ));
            return Err(error);
        }
        handlers.push(handler);
    }
    Ok(Plan { handlers })
}

fn classify_method(method: &ImplItemFn) -> syn::Result<Handler> {
    reject_unsupported_signature(&method.sig)?;

    let ident = method.sig.ident.clone();
    let wire_name = to_camel_case(&ident.to_string());

    let mut has_receiver = false;
    let mut args = Vec::new();
    for input in &method.sig.inputs {
        match input {
            FnArg::Receiver(Receiver {
                mutability, kind, ..
            }) => {
                if !matches!(kind, ReceiverKind::Reference(_, _, None)) || mutability.is_some() {
                    return Err(syn::Error::new(
                        input.span(),
                        "#[tynd::handler] only supports `&self` or no receiver; park mutable state \
                         inside a `LazyLock<Mutex<…>>`",
                    ));
                }
                has_receiver = true;
            },
            FnArg::Typed(PatType { pat, ty, .. }) => {
                let Pat::Ident(pat_ident) = pat.as_ref() else {
                    return Err(syn::Error::new(
                        pat.span(),
                        "#[tynd::handler] only supports plain identifier parameters (`name: T`) — \
                         destructuring patterns aren't expressible in the generated \
                         `serde_json::from_value` call.",
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

    Ok(Handler {
        ident,
        wire_name,
        has_receiver,
        args,
        ret,
    })
}

/// Refuse signatures the generated dispatcher cannot call: `async`,
/// `unsafe`, `const`, `extern`, variadic and generic methods.
fn reject_unsupported_signature(sig: &Signature) -> syn::Result<()> {
    if let Some(token) = sig.asyncness {
        return Err(syn::Error::new(
            token.span(),
            "#[tynd::handler] async methods are unsupported; perform async work behind a \
             synchronous host boundary",
        ));
    }
    if let Safety::Unsafe(token) = &sig.safety {
        return Err(syn::Error::new(
            token.span(),
            "#[tynd::handler] unsafe methods cannot be exposed over IPC",
        ));
    }
    if sig.constness.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
    {
        return Err(syn::Error::new(
            sig.span(),
            "#[tynd::handler] const, extern, variadic, and generic methods are unsupported",
        ));
    }
    Ok(())
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
        cand.len() == segs.len() && cand.iter().zip(segs.iter()).all(|(a, b)| *a == b.as_str())
    })
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
mod tests;
