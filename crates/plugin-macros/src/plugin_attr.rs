//! Expansion for `#[plugin(name = "…")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, spanned::Spanned, ItemStruct};
pub(crate) fn expand_plugin_attr(
    args: TokenStream,
    input: TokenStream,
) -> syn::Result<TokenStream> {
    let item: ItemStruct = parse2(input)?;
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new(
            item.generics.span(),
            "#[tynd_plugin::plugin] requires a plain struct (no generics)",
        ));
    }
    let name = parse_name_arg(args)?;
    let struct_ident = &item.ident;

    Ok(quote! {
        #item

        impl #struct_ident {
            #[doc(hidden)]
            pub const __TYND_PLUGIN_NAME: &'static str = #name;
        }

        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub fn __tynd_plugin() -> ::std::sync::Arc<dyn ::tynd_plugin_sdk::TyndPlugin> {
            ::std::sync::Arc::new(<#struct_ident as ::std::default::Default>::default())
        }
    })
}

fn parse_name_arg(args: TokenStream) -> syn::Result<String> {
    if args.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[plugin] requires `name = \"<wire-name>\"` — e.g. `#[plugin(name = \"sql\")]`",
        ));
    }
    let parsed = parse2::<NameArg>(args)?;
    Ok(parsed.name)
}

struct NameArg {
    name: String,
}

impl syn::parse::Parse for NameArg {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let key: syn::Ident = input.parse()?;
        if key != "name" {
            return Err(syn::Error::new(key.span(), "expected `name = \"…\"`"));
        }
        let _eq: syn::Token![=] = input.parse()?;
        let lit: syn::LitStr = input.parse()?;
        let value = lit.value();
        if value.is_empty() {
            return Err(syn::Error::new(lit.span(), "plugin name must not be empty"));
        }
        if value.chars().any(char::is_whitespace) {
            return Err(syn::Error::new(
                lit.span(),
                "plugin name must not contain whitespace",
            ));
        }
        // The wire key shape `$plugin$<name>$<method>` and the JS-side
        // `${name}:${method}` join must stay unambiguous.
        if let Some(bad) = value
            .chars()
            .find(|c| matches!(*c, '$' | ':' | '/' | ';') || c.is_control())
        {
            return Err(syn::Error::new(
                lit.span(),
                format!(
                    "plugin name must not contain wire-protocol-reserved char {bad:?} (any of `$ \
                     : / ;` or control chars)"
                ),
            ));
        }
        let mut chars = value.chars();
        let canonical = value.len() <= 31
            && chars.next().is_some_and(|ch| ch.is_ascii_lowercase())
            && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
        if !canonical {
            return Err(syn::Error::new(
                lit.span(),
                "plugin name must be canonical lowercase kebab-case: `^[a-z][a-z0-9-]{0,30}$`",
            ));
        }
        if !input.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "unexpected token after `name = \"…\"` — `#[plugin]` only accepts the `name` key",
            ));
        }
        Ok(Self { name: value })
    }
}
