use std::fmt::Write as _;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{spanned::Spanned, Field, Fields, Item, ItemEnum, ItemMod, ItemStruct, Variant};

use crate::analyze::{
    self, auto_number_replacement, has_skip, has_specta_type_attr, missing_derives,
    parse_plugin_command_flags, strip_plugin_command_attrs, Plan,
};

/// Rewrite the user's `mod commands { ... }` and emit the sibling
/// `__tynd_export_command_types_<modname>` helper. Errors surface as
/// compile errors at the macro call site.
pub(crate) fn expand(mut module: ItemMod) -> syn::Result<TokenStream> {
    let plan = analyze::analyze_module(&module);

    if let Some((_, items)) = module.content.as_mut() {
        for item in items.iter_mut() {
            rewrite_item(item)?;
        }
    }

    // Export fn lives *next to* the module so callers can reach it as
    // `crate::__tynd_export_command_types_<modname>()`. Module-name
    // suffix permits multiple `#[commands]` modules in the same crate
    // (one symbol per module). Distinct base name from
    // `__tynd_export_types` emitted by `#[plugin_impl]` so the same
    // crate can use both macros without a duplicate-symbol compile
    // error.
    let export_fn = render_export_fn(&plan);

    Ok(quote! {
        #module

        #export_fn
    })
}

/// Apply derives + field auto-annotations to a single item; non-target
/// items (consts, type aliases, sub-modules) are left untouched.
fn rewrite_item(item: &mut Item) -> syn::Result<()> {
    match item {
        Item::Struct(s) => {
            if has_skip(&s.attrs) {
                strip_skip(&mut s.attrs);
                return Ok(());
            }
            // Parse + strip `#[plugin_command(...)]` before derive
            // injection so the zero_copy flag flows through.
            let flags = parse_plugin_command_flags(&s.attrs)?;
            strip_plugin_command_attrs(&mut s.attrs);
            inject_derives_struct(s, flags.zero_copy);
            annotate_struct_fields(s)?;
        },
        Item::Enum(e) => {
            if has_skip(&e.attrs) {
                strip_skip(&mut e.attrs);
                return Ok(());
            }
            let flags = parse_plugin_command_flags(&e.attrs)?;
            strip_plugin_command_attrs(&mut e.attrs);
            inject_derives_enum(e, flags.zero_copy);
            for variant in &mut e.variants {
                annotate_variant_fields(variant)?;
            }
        },
        _ => {},
    }
    Ok(())
}

fn inject_derives_struct(s: &mut ItemStruct, include_rkyv: bool) {
    let missing = missing_derives(&s.attrs);
    if let Some(extra) = missing.to_tokens(include_rkyv) {
        let attr: syn::Attribute = syn::parse_quote!(#extra);
        // Prepend the derive: the `legacy_derive_helpers` lint rejects
        // `#[serde(...)]` / `#[specta(...)]` helpers placed before the
        // derive that introduces them.
        s.attrs.insert(0, attr);
    }
    // `#[serde(crate = ...)]` points the serde derive at the SDK
    // re-export: without it, serde_derive emits `extern crate serde;`,
    // which fails (E0463) in plugins that only depend on the SDK.
    if missing.needs_serde_crate_attr() && !has_serde_crate_attr(&s.attrs) {
        s.attrs.push(serde_crate_attr());
    }
    if missing.needs_specta_crate_attr() {
        // `#[specta(crate = ::tynd_plugin_sdk::specta)]` lets the specta
        // derive find its runtime types through the SDK re-export so
        // plugin authors don't need a direct `specta` dep. Only emitted
        // when the derive itself was injected — if the user wrote their
        // own `#[derive(specta::Type)]` they're responsible for the path.
        s.attrs.push(specta_crate_attr());
    }
    if include_rkyv {
        // `#[rkyv(crate = …)]` must follow the derive that introduces
        // it (same `legacy_derive_helpers` constraint).
        s.attrs.push(rkyv_crate_attr());
    }
}

fn inject_derives_enum(e: &mut ItemEnum, include_rkyv: bool) {
    let missing = missing_derives(&e.attrs);
    if let Some(extra) = missing.to_tokens(include_rkyv) {
        let attr: syn::Attribute = syn::parse_quote!(#extra);
        e.attrs.insert(0, attr);
    }
    if missing.needs_serde_crate_attr() && !has_serde_crate_attr(&e.attrs) {
        e.attrs.push(serde_crate_attr());
    }
    if missing.needs_specta_crate_attr() {
        e.attrs.push(specta_crate_attr());
    }
    if include_rkyv {
        e.attrs.push(rkyv_crate_attr());
    }
}

/// `#[rkyv(crate = ::tynd_plugin_sdk::rkyv)]` — points the rkyv 0.8
/// derives at the SDK re-export so plugin authors don't need a direct
/// `rkyv` dep.
fn rkyv_crate_attr() -> syn::Attribute {
    syn::parse_quote!(#[rkyv(crate = ::tynd_plugin_sdk::rkyv)])
}

/// `#[serde(crate = "::tynd_plugin_sdk::serde")]` — points the serde
/// derives at the SDK re-export so plugin authors don't need a direct
/// `serde` dep. Mirrors [`rkyv_crate_attr`].
fn serde_crate_attr() -> syn::Attribute {
    syn::parse_quote!(#[serde(crate = "::tynd_plugin_sdk::serde")])
}

/// `true` when the item already carries `#[serde(crate = ...)]`
/// (rustc rejects a duplicate).
fn has_serde_crate_attr(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("serde"))
        .any(|a| {
            let mut found = false;
            let _ = a.parse_nested_meta(|meta| {
                if meta.path.is_ident("crate") {
                    found = true;
                }
                // Consume any `= value` / `(...)` so parsing continues.
                if meta.input.peek(syn::Token![=]) {
                    let _: syn::Expr = meta.value()?.parse()?;
                } else if meta.input.peek(syn::token::Paren) {
                    let _: proc_macro2::Group = meta.input.parse()?;
                }
                Ok(())
            });
            found
        })
}

/// `#[specta(crate = ::tynd_plugin_sdk::specta)]` — points the specta
/// derive at the SDK re-export so plugin authors don't need a direct
/// `specta` dep. Mirrors [`rkyv_crate_attr`].
fn specta_crate_attr() -> syn::Attribute {
    syn::parse_quote!(#[specta(crate = ::tynd_plugin_sdk::specta)])
}

fn annotate_struct_fields(s: &mut ItemStruct) -> syn::Result<()> {
    annotate_fields(&mut s.fields)
}

fn annotate_variant_fields(variant: &mut Variant) -> syn::Result<()> {
    annotate_fields(&mut variant.fields)
}

fn annotate_fields(fields: &mut Fields) -> syn::Result<()> {
    match fields {
        Fields::Named(named) => {
            for field in &mut named.named {
                annotate_field(field)?;
            }
        },
        Fields::Unnamed(unnamed) => {
            for field in &mut unnamed.unnamed {
                annotate_field(field)?;
            }
        },
        Fields::Unit => {},
    }
    Ok(())
}

fn annotate_field(field: &mut Field) -> syn::Result<()> {
    if has_skip(&field.attrs) {
        // `#[skip]` on a field opts out of the u64→f64 rewrite (user
        // wants the bigint TS path). Strip the marker and bail.
        strip_skip(&mut field.attrs);
        return Ok(());
    }
    if has_specta_type_attr(&field.attrs) {
        return Ok(());
    }
    if let Some(replacement) = auto_number_replacement(&field.ty) {
        let attr: syn::Attribute = syn::parse_quote!(#[specta(type = #replacement)]);
        field.attrs.push(attr);
    }
    Ok(())
}

fn strip_skip(attrs: &mut Vec<syn::Attribute>) {
    attrs.retain(|attr| !analyze::is_skip_attr(attr));
}

/// Render the `__tynd_export_command_types_<modname>()` fn that lives
/// next to the rewritten module. Specta walks the `Type` graph from
/// the registered roots so nested types are picked up transitively.
/// The trailer emits a `_commands` const — the catalog consumed by
/// `defineTypedPlugin` in `@tynd/plugin-sdk` for fully-typed
/// transparent IPC.
///
/// The fn ident carries the source-module suffix so a single crate
/// can host multiple `#[tynd_plugin::commands]`-annotated modules
/// (`pub mod foo` + `pub mod bar`) without colliding on a duplicate
/// `__tynd_export_command_types` symbol at link time.
fn render_export_fn(plan: &Plan) -> TokenStream {
    let mod_ident = &plan.mod_ident;
    let export_fn_ident = quote::format_ident!("__tynd_export_command_types_{}", mod_ident);
    let registers = plan.exported_idents.iter().map(|ident| {
        quote! {
            let types = types.register::< #mod_ident :: #ident >();
        }
    });
    let catalog = render_command_catalog(plan);

    quote! {
        /// TypeScript declarations for every wire-shape in the module
        /// + a `_commands` catalog for `defineTypedPlugin`, as one
        /// string ready to be written to a `.gen.ts` file.
        #[doc(hidden)]
        #[allow(unused_qualifications)]
        pub fn #export_fn_ident() -> ::std::string::String {
            let types = ::tynd_plugin_sdk::specta::Types::default();
            #(#registers)*
            // `specta_serde::PhasesFormat` reads `#[serde(rename_all = ...)]`
            // and rewrites the resolved type graph. specta-typescript
            // by itself prints raw Rust idents — without this hook a
            // `success_html: String` field would land as `success_html`
            // in `.gen.ts` while serde sends `successHtml` at runtime.
            // `PhasesFormat` (not the unified `Format`) because plugin authors
            // routinely use phase-asymmetric serde attrs
            // (`skip_serializing_if`, `default`); the unified format rejects
            // those. `PhasesFormat` accepts them by splitting the graph when
            // needed.
            let mut out = ::tynd_plugin_sdk::specta_typescript::Typescript::default()
                .export(&types, ::tynd_plugin_sdk::specta_serde::PhasesFormat)
                .expect("specta-typescript export failed (unreachable in well-formed code)");
            out.push_str(#catalog);
            out
        }
    }
}

/// Render the TS `_commands` catalog block as a string literal.
/// Includes hand-emitted aliases for `pub type X = …;` items because
/// specta only walks `#[derive(Type)]` items — type aliases are
/// invisible to it. Empty when no `<Stem>Args` pairs were detected.
fn render_command_catalog(plan: &Plan) -> String {
    if plan.commands.is_empty() && plan.type_aliases.is_empty() {
        return String::new();
    }
    let mut s = String::new();

    if !plan.type_aliases.is_empty() {
        for alias in &plan.type_aliases {
            let _ = writeln!(
                s,
                "export type {ident} = {target};",
                ident = alias.ident,
                target = alias.ts_target,
            );
        }
    }

    if !plan.commands.is_empty() {
        s.push_str("export const _commands = {\n");
        for cmd in &plan.commands {
            let args = cmd.args_ident.to_string();
            let result = match &cmd.result_ident {
                Some(r) => format!("undefined as unknown as {r}"),
                None => "undefined as unknown as null".to_string(),
            };
            let _ = writeln!(
                s,
                "  {method}: {{ args: undefined as unknown as {args}, result: {result} }},",
                method = cmd.method,
            );
        }
        s.push_str("} as const;\n");
        s.push_str("export type CommandsCatalog = typeof _commands;\n");
    }
    s
}

/// Friendly error pointing at the user's input span.
pub(crate) fn err_expected_mod(input: &impl Spanned) -> syn::Error {
    syn::Error::new(
        input.span(),
        "#[tynd_plugin::commands] expected `mod` — the macro must be placed on a module \
         declaration like `pub mod commands { ... }`",
    )
}

#[cfg(test)]
mod tests {
    use quote::ToTokens;

    use super::{inject_derives_enum, inject_derives_struct};

    fn serde_crate_count(attrs: &[syn::Attribute]) -> usize {
        attrs
            .iter()
            .map(|a| a.to_token_stream().to_string())
            .filter(|s| s.contains("serde") && s.contains("crate = \"::tynd_plugin_sdk::serde\""))
            .count()
    }

    #[test]
    fn injected_serde_derive_points_at_sdk_reexport() {
        let mut s: syn::ItemStruct = syn::parse_quote!(
            pub struct Foo {
                a: String,
            }
        );
        inject_derives_struct(&mut s, false);
        assert_eq!(serde_crate_count(&s.attrs), 1);

        let mut e: syn::ItemEnum = syn::parse_quote!(
            pub enum Bar {
                A,
            }
        );
        inject_derives_enum(&mut e, false);
        assert_eq!(serde_crate_count(&e.attrs), 1);
    }

    #[test]
    fn existing_serde_crate_attr_is_not_duplicated() {
        let mut s: syn::ItemStruct = syn::parse_quote!(
            #[derive(Debug)]
            #[serde(rename_all = "camelCase", crate = "::tynd_plugin_sdk::serde")]
            pub struct Foo {
                a: String,
            }
        );
        inject_derives_struct(&mut s, false);
        assert_eq!(serde_crate_count(&s.attrs), 1);
    }

    #[test]
    fn user_written_serde_derives_get_no_crate_attr() {
        let mut s: syn::ItemStruct = syn::parse_quote!(
            #[derive(serde::Serialize, serde::Deserialize)]
            pub struct Foo {
                a: String,
            }
        );
        inject_derives_struct(&mut s, false);
        assert_eq!(serde_crate_count(&s.attrs), 0);
    }
}
