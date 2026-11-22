//! `#[plugin]`, `#[plugin_impl]`, `#[command]` — generate the full TyndPlugin
//! impl + Args/Result types + specta exporter from a typed Rust impl block.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse2, spanned::Spanned, Attribute, FnArg, ImplItem, ItemImpl, ReturnType};

mod command;

use command::{
    analyse_command_fn, extract_ok_type, is_plugin_cx_ref, render_args_struct, render_dispatch_arm,
    render_export_fn, render_result_alias, type_ident, CommandFn,
};

struct PluginImplAnalysis {
    commands: Vec<CommandFn>,
    has_plugin_init: bool,
    has_plugin_shutdown: bool,
}

pub(crate) fn expand_plugin_impl(
    _args: TokenStream,
    input: TokenStream,
) -> syn::Result<TokenStream> {
    let mut impl_item: ItemImpl = parse2(input)?;
    let self_ty = &impl_item.self_ty;
    let self_ident = type_ident(self_ty).cloned().ok_or_else(|| {
        syn::Error::new(
            self_ty.span(),
            "#[plugin_impl] requires a plain struct type (no generics, no dyn)",
        )
    })?;
    let analysis = analyse_plugin_methods(&mut impl_item)?;
    Ok(render_plugin_impl(&impl_item, &self_ident, analysis))
}

fn analyse_plugin_methods(impl_item: &mut ItemImpl) -> syn::Result<PluginImplAnalysis> {
    let mut analysis = PluginImplAnalysis {
        commands: Vec::new(),
        has_plugin_init: false,
        has_plugin_shutdown: false,
    };
    for item in &mut impl_item.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let was_command = method.attrs.iter().any(is_command_attr);
        let lifecycle_hook = match method.sig.ident.to_string().as_str() {
            "plugin_init" => Some("init"),
            "plugin_shutdown" => Some("shutdown"),
            _ => None,
        };
        if let Some(hook) = lifecycle_hook {
            if was_command {
                return Err(syn::Error::new(
                    method.sig.ident.span(),
                    format!(
                        "`plugin_{hook}` is a native lifecycle hook and cannot be tagged \
                         `#[command]`"
                    ),
                ));
            }
            validate_lifecycle_hook(method, hook)?;
            match hook {
                "init" => analysis.has_plugin_init = true,
                "shutdown" => analysis.has_plugin_shutdown = true,
                _ => unreachable!("lifecycle hook names are fixed above"),
            }
        }
        if was_command {
            method.attrs.retain(|attribute| !is_command_attr(attribute));
            analysis.commands.push(analyse_command_fn(method)?);
        }
    }
    if analysis.commands.is_empty() {
        return Err(syn::Error::new(
            impl_item.span(),
            "#[plugin_impl] expects at least one method tagged `#[command]`",
        ));
    }
    Ok(analysis)
}

fn render_plugin_impl(
    impl_item: &ItemImpl,
    self_ident: &syn::Ident,
    analysis: PluginImplAnalysis,
) -> TokenStream {
    let commands = analysis.commands;
    let args_structs = commands.iter().map(render_args_struct);
    let result_aliases = commands.iter().map(render_result_alias);
    let dispatch_arms = commands.iter().map(render_dispatch_arm);
    let export_fn = render_export_fn(self_ident, &commands);
    let names: Vec<&str> = commands.iter().map(CommandFn::method_name).collect();
    let init_hook = analysis
        .has_plugin_init
        .then(|| render_init_hook(self_ident));
    let shutdown_hook = analysis
        .has_plugin_shutdown
        .then(|| render_shutdown_hook(self_ident));

    quote! {
        #impl_item

        #(#args_structs)*
        #(#result_aliases)*

        // Surface a `Send + Sync + 'static` failure at the user's
        // struct span instead of inside macro-emitted code — without
        // this, a non-Send field reports the auto-trait error at the
        // generated `impl TyndPlugin` line with no hint at the field.
        const _: fn() = || {
            fn __tynd_plugin_assert_send_sync_static<
                T: ::core::marker::Send + ::core::marker::Sync + 'static,
            >() {
            }
            __tynd_plugin_assert_send_sync_static::<#self_ident>();
        };

        impl ::tynd_plugin_sdk::TyndPlugin for #self_ident {
            fn name(&self) -> &'static str {
                <#self_ident>::__TYND_PLUGIN_NAME
            }

            fn command_names(&self) -> &'static [&'static str] {
                <#self_ident>::__TYND_COMMAND_NAMES
            }

            #init_hook

            #[allow(unused_variables)]
            fn dispatch(
                &self,
                __tynd_plugin_cx: &::tynd_plugin_sdk::PluginCx,
                __tynd_plugin_method: &str,
                __tynd_plugin_args_value: ::tynd_plugin_sdk::serde_json::Value,
            ) -> ::tynd_plugin_sdk::PluginResult<::tynd_plugin_sdk::serde_json::Value> {
                match __tynd_plugin_method {
                    #(#dispatch_arms),* ,
                    _ => Err(::tynd_plugin_sdk::unknown_method(
                        <#self_ident>::__TYND_PLUGIN_NAME,
                        __tynd_plugin_method,
                    )),
                }
            }

            #shutdown_hook
        }

        #export_fn

        impl #self_ident {
            #[doc(hidden)]
            #[allow(dead_code)]
            pub const __TYND_COMMAND_NAMES: &'static [&'static str] = &[#(#names),*];
        }
    }
}

fn render_init_hook(self_ident: &syn::Ident) -> TokenStream {
    quote! {
        fn init(
            &self,
            __tynd_plugin_cx: &::tynd_plugin_sdk::PluginCx,
        ) -> ::tynd_plugin_sdk::PluginResult<()> {
            #self_ident::plugin_init(self, __tynd_plugin_cx)
        }
    }
}

fn render_shutdown_hook(self_ident: &syn::Ident) -> TokenStream {
    quote! {
        fn shutdown(
            &self,
            __tynd_plugin_cx: &::tynd_plugin_sdk::PluginCx,
        ) -> ::tynd_plugin_sdk::PluginResult<()> {
            #self_ident::plugin_shutdown(self, __tynd_plugin_cx)
        }
    }
}
/// Validate the two reserved inherent lifecycle methods consumed by
/// `#[plugin_impl]`. Keeping these names outside the renderer command namespace
/// lets typed plugins customize `TyndPlugin::{init,shutdown}` without writing a
/// second, conflicting trait impl.
fn validate_lifecycle_hook(method: &syn::ImplItemFn, hook: &str) -> syn::Result<()> {
    let signature = &method.sig;
    if signature.asyncness.is_some()
        || signature.constness.is_some()
        || matches!(signature.safety, syn::Safety::Unsafe(_))
        || signature.abi.is_some()
        || !signature.generics.params.is_empty()
        || signature.variadic.is_some()
    {
        return Err(syn::Error::new(
            signature.span(),
            format!(
                "`plugin_{hook}` must be a plain synchronous method: \
                 `fn plugin_{hook}(&self, cx: &PluginCx) -> PluginResult<()>`"
            ),
        ));
    }

    let mut inputs = signature.inputs.iter();
    let receiver_valid = matches!(
        inputs.next(),
        Some(FnArg::Receiver(receiver))
            if matches!(receiver.kind, syn::ReceiverKind::Reference(_, _, None))
                && receiver.mutability.is_none()
    );
    let context_valid = matches!(
        inputs.next(),
        Some(FnArg::Typed(context)) if is_plugin_cx_ref(&context.ty)
    );
    if !receiver_valid || !context_valid || inputs.next().is_some() {
        return Err(syn::Error::new(
            signature.inputs.span(),
            format!("`plugin_{hook}` must take exactly `&self, cx: &PluginCx`"),
        ));
    }

    let ReturnType::Type(_, output) = &signature.output else {
        return Err(syn::Error::new(
            signature.output.span(),
            format!("`plugin_{hook}` must return `PluginResult<()>`"),
        ));
    };
    match extract_ok_type(output) {
        Ok(None) => Ok(()),
        _ => Err(syn::Error::new(
            output.span(),
            format!("`plugin_{hook}` must return `PluginResult<()>`"),
        )),
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────

fn is_command_attr(a: &Attribute) -> bool {
    let segs: Vec<String> = a
        .path()
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    matches!(
        segs.as_slice(),
        [s] if s == "command"
    ) || matches!(
        segs.as_slice(),
        [a, b] if (a == "tynd_plugin" || a == "tynd_plugin_macros") && b == "command"
    )
}
