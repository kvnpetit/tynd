//! Per-command IR, signature analysis and generated dispatch/types.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    spanned::Spanned, Attribute, FnArg, GenericArgument, Pat, PathArguments, ReturnType, Type,
    TypePath,
};

use crate::{
    analyze::{auto_number_replacement, render_ts_type},
    util::{pascal_to_camel, snake_to_pascal},
};
// ─── Per-command analysis ─────────────────────────────────────────────

#[derive(Debug)]
pub(super) struct CommandFn {
    /// Original Rust fn ident (snake_case).
    fn_ident: syn::Ident,
    /// Wire method name (camelCase of the fn ident).
    method_name: String,
    /// `<MethodName>Args` struct ident.
    args_ident: syn::Ident,
    /// `<MethodName>Result` alias ident.
    result_ident: syn::Ident,
    /// Fields of the Args struct: `(param_ident, param_type)` tuples.
    /// Excludes `&self` and an optional leading `cx: &PluginCx`.
    fields: Vec<(syn::Ident, Type)>,
    /// `true` if the fn signature starts with `cx: &PluginCx` (after self).
    takes_cx: bool,
    /// `true` if `async fn` — the dispatch arm must `.await`.
    is_async: bool,
    /// `None` means the fn returned `PluginResult<()>`.
    ok_type: Option<Type>,
    docs: Vec<Attribute>,
}

impl CommandFn {
    pub(super) fn method_name(&self) -> &str {
        &self.method_name
    }

    /// True when the return type already carries the `<Method>Result` name:
    /// emitting the alias would self-loop (`pub type StartResult = StartResult;`).
    fn returns_result_alias(&self) -> bool {
        self.ok_type
            .as_ref()
            .and_then(type_ident)
            .is_some_and(|ret_ident| ret_ident == &self.result_ident)
    }
}

pub(super) fn analyse_command_fn(method: &syn::ImplItemFn) -> syn::Result<CommandFn> {
    let fn_ident = method.sig.ident.clone();
    let pascal = snake_to_pascal(&fn_ident.to_string());
    let method_name = pascal_to_camel(&pascal);
    let args_ident = format_ident!("{}Args", pascal);
    let result_ident = format_ident!("{}Result", pascal);

    let mut inputs = method.sig.inputs.iter().peekable();
    check_receiver(method, inputs.next())?;
    let takes_cx = inputs.next_if(|arg| is_cx_param(arg)).is_some();
    let fields = collect_fields(inputs)?;

    let ok_type = match &method.sig.output {
        ReturnType::Default => {
            return Err(syn::Error::new(method.sig.span(), RETURN_TYPE_ERROR));
        },
        ReturnType::Type(_, ty) => extract_ok_type(ty)?,
    };

    let docs = method
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .cloned()
        .collect();

    Ok(CommandFn {
        fn_ident,
        method_name,
        args_ident,
        result_ident,
        fields,
        takes_cx,
        is_async: method.sig.asyncness.is_some(),
        ok_type,
        docs,
    })
}

const RETURN_TYPE_ERROR: &str =
    "tynd #[command] returns must be `PluginResult<T>` (alias for `Result<T, PluginError>`)";

/// Accept only a `&self` receiver in first position.
fn check_receiver(method: &syn::ImplItemFn, first: Option<&FnArg>) -> syn::Result<()> {
    match first {
        Some(FnArg::Receiver(recv))
            if matches!(recv.kind, syn::ReceiverKind::Reference(_, _, None))
                && recv.mutability.is_none() =>
        {
            Ok(())
        },
        // Reject `&mut self`, `self`, `self: Box<Self>` etc. up front —
        // `TyndPlugin::dispatch` takes `&self`, and the mismatch otherwise
        // surfaces inside macro-emitted code with a span the user can't act on.
        Some(FnArg::Receiver(recv)) => Err(syn::Error::new(
            recv.span(),
            "#[command] requires a `&self` receiver (TyndPlugin::dispatch is `&self`)",
        )),
        _ => Err(syn::Error::new(
            method.sig.span(),
            "#[command] methods need a `&self` receiver",
        )),
    }
}

/// The optional `cx: &PluginCx` second arg: matched on the canonical names
/// `cx`/`ctx` OR on a `&PluginCx` type — so `_cx: SomeOtherType` does not
/// get silently dropped.
fn is_cx_param(arg: &FnArg) -> bool {
    let FnArg::Typed(pat_type) = arg else {
        return false;
    };
    is_plugin_cx_ref(&pat_type.ty)
        || matches!(
            pat_type.pat.as_ref(),
            Pat::Ident(p) if p.ident == "cx" || p.ident == "ctx"
        )
}

/// Remaining params become the Args struct fields, in declaration order.
fn collect_fields<'a>(
    inputs: impl Iterator<Item = &'a FnArg>,
) -> syn::Result<Vec<(syn::Ident, Type)>> {
    inputs
        .map(|arg| match arg {
            FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
                Pat::Ident(p) => Ok((p.ident.clone(), (*pat_type.ty).clone())),
                other => Err(syn::Error::new(
                    other.span(),
                    "#[command] params must be plain `name: Type` bindings",
                )),
            },
            FnArg::Receiver(_) => Err(syn::Error::new(
                arg.span(),
                "#[command] methods may have only one receiver, at position 0",
            )),
        })
        .collect()
}

/// Pull `T` out of `PluginResult<T>`. Only `PluginResult<T>` is accepted
/// because the SDK ships a single `IntoValueRoute` impl for it; rejecting
/// here surfaces the constraint at the user's span instead of as a trait
/// error inside macro-generated code. `PluginResult<()>` → `Ok(None)`.
pub(super) fn extract_ok_type(ty: &Type) -> syn::Result<Option<Type>> {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        if let Some(last) = path.segments.last() {
            let name = last.ident.to_string();
            if name == "PluginResult" {
                if let PathArguments::AngleBracketed(ab) = &last.arguments {
                    for arg in &ab.args {
                        if let GenericArgument::Type(t) = arg {
                            return Ok(if is_unit_type(t) {
                                None
                            } else {
                                Some(t.clone())
                            });
                        }
                    }
                }
            }
        }
    }
    Err(syn::Error::new(ty.span(), RETURN_TYPE_ERROR))
}

fn is_unit_type(ty: &Type) -> bool {
    matches!(ty, Type::Tuple(t) if t.elems.is_empty())
}

pub(super) fn render_args_struct(cmd: &CommandFn) -> TokenStream {
    let docs = &cmd.docs;
    let ident = &cmd.args_ident;
    let attrs = quote! {
        #(#docs)*
        #[derive(Debug, Clone, ::tynd_plugin_sdk::serde::Serialize, ::tynd_plugin_sdk::serde::Deserialize, ::tynd_plugin_sdk::specta::Type)]
        #[specta(crate = ::tynd_plugin_sdk::specta)]
        #[serde(crate = "::tynd_plugin_sdk::serde", rename_all = "camelCase")]
    };
    if cmd.fields.is_empty() {
        // Unit struct (not bracketed) so `null` deserialises cleanly.
        return quote! {
            #attrs
            pub struct #ident;
        };
    }
    // Re-route 64-bit ints through `f64` so specta-typescript doesn't
    // reject the BigInt-forbidden primitives.
    let field_decls = cmd.fields.iter().map(|(name, ty)| {
        if let Some(repl) = auto_number_replacement(ty) {
            quote! {
                #[specta(type = #repl)]
                pub #name: #ty
            }
        } else {
            quote! { pub #name: #ty }
        }
    });
    quote! {
        #attrs
        pub struct #ident {
            #(#field_decls),*
        }
    }
}

pub(super) fn render_result_alias(cmd: &CommandFn) -> TokenStream {
    let ident = &cmd.result_ident;
    if cmd.returns_result_alias() {
        return quote! {};
    }
    if let Some(ty) = &cmd.ok_type {
        quote! {
            #[allow(dead_code)]
            pub type #ident = #ty;
        }
    } else {
        quote! {
            #[allow(dead_code)]
            pub type #ident = ();
        }
    }
}

pub(super) fn render_dispatch_arm(cmd: &CommandFn) -> TokenStream {
    let method = &cmd.method_name;
    let fn_ident = &cmd.fn_ident;
    let args_ident = &cmd.args_ident;

    let cx_arg = if cmd.takes_cx {
        quote! { __tynd_plugin_cx, }
    } else {
        quote! {}
    };
    let field_pass = cmd
        .fields
        .iter()
        .map(|(name, _)| quote! { __tynd_plugin_args.#name });
    let mut call = quote! { self.#fn_ident(#cx_arg #(#field_pass),*) };
    if cmd.is_async {
        // See `__internal::block_on` for the no-tokio-runtime contract.
        call = quote! {
            ::tynd_plugin_sdk::__internal::block_on(async move { #call.await })
        };
    }

    // Every binding emitted here is prefixed `__tynd_plugin_*` so user
    // struct fields, method parameters, or in-scope idents named `args`,
    // `cx`, `e`, `out` etc. cannot be silently shadowed by the dispatch
    // arm. Macro hygiene foot-gun otherwise.
    let from_value = quote! {
        ::tynd_plugin_sdk::serde_json::from_value(__tynd_plugin_args_value)
            .map_err(|__tynd_plugin_de_err| ::tynd_plugin_sdk::PluginError::InvalidArgsBlob {
                method: #method.to_string(),
                detail: __tynd_plugin_de_err.to_string(),
            })?
    };
    let parse = if cmd.fields.is_empty() {
        quote! {
            if !__tynd_plugin_args_value.is_null() {
                let _: #args_ident = #from_value;
            }
        }
    } else {
        quote! {
            let __tynd_plugin_args: #args_ident = #from_value;
        }
    };

    quote! {
        #method => {
            #parse
            let __tynd_plugin_out = #call;
            ::tynd_plugin_sdk::__internal::IntoValueRoute::into_value_route(__tynd_plugin_out)
        }
    }
}

pub(super) fn render_export_fn(self_ident: &syn::Ident, commands: &[CommandFn]) -> TokenStream {
    // Skip primitive ok-types: specta-typescript rejects bare `u64` and
    // primitive `Vec<T>` resolves the same way as its inner; the TS
    // alias trailer renders those directly.
    let registers = commands.iter().flat_map(|cmd| {
        let args = &cmd.args_ident;
        let mut tokens = vec![quote! { let types = types.register::< #args >(); }];
        if let Some(ok) = &cmd.ok_type {
            if !is_primitive_like(ok) {
                tokens.push(quote! { let types = types.register::< #ok >(); });
            }
        }
        tokens
    });

    // specta doesn't see `#[command]` fns; hand-emit the catalog in the
    // same shape the `#[commands]` mod produces.
    let catalog_lines = commands.iter().map(|cmd| {
        let method = &cmd.method_name;
        let args = cmd.args_ident.to_string();
        let result = cmd.result_ident.to_string();
        let fragment = format!(
            "  {method}: {{ args: undefined as unknown as {args}, result: undefined as unknown as \
             {result} }},\n"
        );
        quote! { __out.push_str(#fragment); }
    });

    let alias_lines = commands.iter().filter_map(|cmd| {
        if cmd.returns_result_alias() {
            return None;
        }
        let alias = cmd.result_ident.to_string();
        let target_ts = cmd
            .ok_type
            .as_ref()
            .map_or_else(|| "null".to_string(), render_ts_type);
        let line = format!("export type {alias} = {target_ts};\n");
        Some(quote! { __out.push_str(#line); })
    });

    quote! {
        impl #self_ident {
            #[doc(hidden)]
            #[allow(unused_qualifications)]
            pub fn __tynd_export_types_impl() -> ::std::string::String {
                let types = ::tynd_plugin_sdk::specta::Types::default();
                #(#registers)*
                // See `codegen.rs` for the `PhasesFormat` rationale.
                let mut __out = ::tynd_plugin_sdk::specta_typescript::Typescript::default()
                    .export(&types, ::tynd_plugin_sdk::specta_serde::PhasesFormat)
                    .expect("specta-typescript export failed");
                #(#alias_lines)*
                __out.push_str("export const _commands = {\n");
                #(#catalog_lines)*
                __out.push_str("} as const;\nexport type CommandsCatalog = typeof _commands;\n");
                __out
            }
        }

        // `gen_types_bin!` reaches the exporter through this free fn
        // regardless of which struct hosts the logic.
        #[doc(hidden)]
        pub fn __tynd_export_types() -> ::std::string::String {
            <#self_ident>::__tynd_export_types_impl()
        }
    }
}

/// True when the type would be rendered to a primitive TS shape
/// (number / boolean / string / nullable / array of same). Used to
/// skip specta registration that would otherwise reject `u64` etc.
///
/// Works on the path's last segment, so generic forms like `Vec<u64>`
/// are recognised — `path.get_ident()` only matches non-generic paths.
fn is_primitive_like(ty: &Type) -> bool {
    let Some(ident) = last_segment_ident(ty) else {
        // Non-path types (refs, tuples) — fall through to specta.
        return false;
    };
    let name = ident.to_string();
    if matches!(
        name.as_str(),
        "u8" | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "f32"
            | "f64"
            | "bool"
            | "String"
            | "str"
    ) {
        return true;
    }
    // `Vec<T>`/`Option<T>`/`HashMap<K,V>`/`BTreeMap<K,V>` only count as
    // primitive-leafs when *every* generic arg is primitive — otherwise
    // a custom value type (e.g. `HashMap<String, MyStruct>`) would skip
    // specta registration and dangle in the TS output.
    if matches!(
        name.as_str(),
        "Vec" | "Option" | "HashMap" | "BTreeMap" | "HashSet" | "BTreeSet"
    ) {
        return all_generic_args_primitive(ty);
    }
    false
}

/// True when *every* generic arg of `ty` is primitive-like. An empty
/// generic list returns false — that shape shouldn't reach this helper
/// because the caller has already matched a known wrapper ident.
fn all_generic_args_primitive(ty: &Type) -> bool {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        if let Some(last) = path.segments.last() {
            if let PathArguments::AngleBracketed(args) = &last.arguments {
                let mut saw_any = false;
                for arg in &args.args {
                    if let GenericArgument::Type(t) = arg {
                        saw_any = true;
                        if !is_primitive_like(t) {
                            return false;
                        }
                    }
                }
                return saw_any;
            }
        }
    }
    false
}

pub(super) fn type_ident(ty: &Type) -> Option<&syn::Ident> {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        path.get_ident()
    } else {
        None
    }
}

/// Last-segment ident of a path type — `Vec<u64>` → `Vec`, `Foo` → `Foo`.
/// Unlike [`type_ident`], works through generics (where `path.get_ident()`
/// returns `None`).
fn last_segment_ident(ty: &Type) -> Option<&syn::Ident> {
    if let Type::Path(TypePath {
        path, qself: None, ..
    }) = ty
    {
        path.segments.last().map(|seg| &seg.ident)
    } else {
        None
    }
}

/// True when `ty` is a reference to a path ending in `PluginCx`.
pub(super) fn is_plugin_cx_ref(ty: &Type) -> bool {
    if let Type::Reference(r) = ty {
        if let Some(ident) = last_segment_ident(&r.elem) {
            return ident == "PluginCx";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{analyse_command_fn, render_args_struct};

    #[test]
    fn args_struct_points_serde_at_sdk_reexport() {
        for method in [
            syn::parse_quote!(
                pub fn ping(&self) -> PluginResult<String> {
                    todo!()
                }
            ),
            syn::parse_quote!(
                pub fn open(&self, path: String) -> PluginResult<u64> {
                    todo!()
                }
            ),
        ] {
            let cmd = analyse_command_fn(&method).unwrap();
            let out = render_args_struct(&cmd).to_string();
            assert!(
                out.contains("crate = \"::tynd_plugin_sdk::serde\""),
                "missing serde crate redirect: {out}"
            );
        }
    }
}
