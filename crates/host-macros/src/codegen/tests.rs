//! Expansion tests: the emitted token stream is re-parsed with `syn` and
//! inspected structurally (dispatch fn present, one arm per handler, skip
//! attributes stripped) plus targeted substring checks on the arm bodies.

use syn::{parse_quote, File, Item, ItemImpl};

use super::*;

fn expand_file(item: ItemImpl) -> File {
    let tokens = expand("probe", item).expect("expansion");
    syn::parse2(tokens).expect("expansion re-parses as Rust items")
}

fn compact(tokens: &impl quote::ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<String>()
}

fn sample() -> ItemImpl {
    parse_quote! {
        impl Probe {
            pub fn read_text(&self, file_path: String, max_bytes: Option<u64>) -> Result<String, String> {
                Ok(file_path)
            }
            pub fn count() -> u32 { 1 }
            pub fn ping() {}
            #[tynd::skip]
            pub fn hidden() {}
        }
    }
}

#[test]
fn emits_original_impl_and_dispatch_fn() {
    let file = expand_file(sample());
    assert_eq!(file.items.len(), 2);

    let Item::Impl(imp) = &file.items[0] else {
        panic!("first item must be the impl block");
    };
    assert_eq!(
        imp.items.len(),
        4,
        "all methods are preserved, including skipped ones"
    );
    let skip_left = imp.items.iter().any(|item| match item {
        syn::ImplItem::Fn(f) => f.attrs.iter().any(analyze::is_skip_attr),
        _ => false,
    });
    assert!(!skip_left, "#[tynd::skip] markers are stripped");
    assert!(
        imp.attrs.iter().any(|attr| attr.path().is_ident("allow")
            && quote::ToTokens::to_token_stream(attr)
                .to_string()
                .contains("unreachable_pub")),
        "`pub` handler methods are exempt from `unreachable_pub`"
    );

    let Item::Fn(dispatch) = &file.items[1] else {
        panic!("second item must be the dispatch fn");
    };
    assert_eq!(dispatch.sig.ident, "dispatch");
    assert!(matches!(dispatch.vis, syn::Visibility::Restricted(_)));
    assert_eq!(dispatch.sig.inputs.len(), 2);
}

#[test]
fn one_arm_per_handler_plus_fallback() {
    let file = expand_file(sample());
    let body = compact(&file.items[1]);
    for wire in ["\"readText\"=>", "\"count\"=>", "\"ping\"=>"] {
        assert_eq!(body.matches(wire).count(), 1, "{wire} in {body}");
    }
    assert!(!body.contains("\"hidden\""), "skipped method has no arm");
    assert!(
        body.contains("IpcError::unknown_method(\"probe\",method)"),
        "{body}"
    );
}

#[test]
fn required_and_optional_args_use_camel_case_keys() {
    let body = compact(&expand_file(sample()).items[1]);
    assert!(
        body.contains("IpcError::missing_arg(\"probe\",\"readText\",\"filePath\")"),
        "required arg errors with missing_arg: {body}"
    );
    assert!(
        !body.contains("missing_arg(\"probe\",\"readText\",\"maxBytes\")"),
        "optional arg never errors as missing"
    );
    assert!(body.contains("args.get(\"maxBytes\")"), "{body}");
    assert!(body.contains("::core::option::Option::None=>::core::option::Option::None"));
}

#[test]
fn receiver_and_return_shapes_drive_invocation() {
    let body = compact(&expand_file(sample()).items[1]);
    assert!(
        body.contains(
            "<Probeas::core::default::Default>::default().read_text(file_path,max_bytes)"
        ),
        "&self methods build a Default receiver: {body}"
    );
    assert!(body.contains("<Probe>::count()"), "{body}");
    assert!(
        body.contains("<Probe>::ping();::core::result::Result::Ok(::serde_json::Value::Null)"),
        "unit returns map to Value::Null: {body}"
    );
}

#[test]
fn empty_impl_still_emits_fallback_only_dispatch() {
    let file = expand_file(parse_quote!(impl Probe {}));
    let body = compact(&file.items[1]);
    assert!(body.contains("matchmethod{_=>"), "{body}");
}

#[test]
fn analysis_errors_propagate_from_expand() {
    let e = expand("probe", parse_quote!(impl Probe { pub fn f(&mut self) {} }))
        .expect_err("invalid receiver")
        .to_string();
    assert!(e.contains("&self"), "{e}");
}

#[test]
fn compile_error_tokens_are_emitted_for_bad_input() {
    let err = expand("probe", parse_quote!(impl Probe { pub async fn f() {} })).unwrap_err();
    let tokens = compact(&err.to_compile_error());
    assert!(tokens.contains("compile_error!"), "{tokens}");
}
