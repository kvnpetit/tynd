//! Analysis-pass tests: attribute parsing, method classification, camelCase
//! mapping and every rejection path, driven by `syn::parse_quote!` samples.

use quote::quote;
use syn::{parse_quote, ItemImpl};

use super::*;

fn plan(item: ItemImpl) -> syn::Result<Plan> {
    analyze_impl(&item.items)
}

fn err(item: ItemImpl) -> String {
    plan(item).expect_err("analysis should fail").to_string()
}

#[test]
fn api_arg_accepts_alphanumeric_literal() {
    assert_eq!(parse_api_arg(quote!(api = "fs")).unwrap(), "fs");
    assert_eq!(parse_api_arg(quote!(api = "store2")).unwrap(), "store2");
}

#[test]
fn api_arg_rejections() {
    let cases = [
        (quote!(), "requires `api"),
        (quote!(api), "single name-value pair"),
        (quote!(name = "fs"), "unexpected key"),
        (quote!(api = 5), "must be a string literal"),
        (quote!(api = ""), "must start with an ASCII letter"),
        (quote!(api = "1fs"), "must start with an ASCII letter"),
        (quote!(api = "my-api"), "only ASCII letters or digits"),
        (quote!(api = "my_api"), "only ASCII letters or digits"),
        (quote!(api = "fs.read"), "only ASCII letters or digits"),
    ];
    for (tokens, needle) in cases {
        let shown = tokens.to_string();
        let e = parse_api_arg(tokens).expect_err(&shown).to_string();
        assert!(e.contains(needle), "{shown}: {e}");
    }
    assert!(parse_api_arg(quote!(api = "fs", extra = 1)).is_err());
}

#[test]
fn camel_case_mapping() {
    assert_eq!(to_camel_case("read_text"), "readText");
    assert_eq!(to_camel_case("a_b_c"), "aBC");
    assert_eq!(to_camel_case("already"), "already");
    assert_eq!(to_camel_case("alreadyCamel"), "alreadyCamel");
    assert_eq!(to_camel_case("trailing_"), "trailing");
    assert_eq!(to_camel_case("double__under"), "doubleUnder");
    assert_eq!(to_camel_case("v2_api"), "v2Api");
}

#[test]
fn classifies_methods_args_and_returns() {
    let p = plan(parse_quote! {
        impl Api {
            pub fn read_text(&self, file_path: String, max_bytes: Option<u64>) -> Result<String, String> { todo!() }
            pub fn plain(n: u32) -> u32 { n }
            pub fn unit_explicit() -> () {}
            pub fn unit_default() {}
            pub fn qualified(x: std::option::Option<u8>) -> core::result::Result<(), String> { Ok(()) }
            fn private_helper() {}
            pub(crate) fn crate_visible() {}
            const LIMIT: u32 = 1;
        }
    })
    .unwrap();

    let names: Vec<_> = p.handlers.iter().map(|h| h.wire_name.as_str()).collect();
    assert_eq!(
        names,
        [
            "readText",
            "plain",
            "unitExplicit",
            "unitDefault",
            "qualified"
        ]
    );

    let read = &p.handlers[0];
    assert_eq!(read.ident, "read_text");
    assert!(read.has_receiver);
    assert!(matches!(read.ret, HandlerReturn::Result));
    let keys: Vec<_> = read
        .args
        .iter()
        .map(|a| (a.wire_key.as_str(), a.optional))
        .collect();
    assert_eq!(keys, [("filePath", false), ("maxBytes", true)]);

    assert!(!p.handlers[1].has_receiver);
    assert!(matches!(p.handlers[1].ret, HandlerReturn::Plain));
    assert!(matches!(p.handlers[2].ret, HandlerReturn::Unit));
    assert!(matches!(p.handlers[3].ret, HandlerReturn::Unit));
    assert!(
        p.handlers[4].args[0].optional,
        "path-qualified Option is optional"
    );
    assert!(matches!(p.handlers[4].ret, HandlerReturn::Result));
}

#[test]
fn skip_markers_hide_methods() {
    let p = plan(parse_quote! {
        impl Api {
            #[tynd::skip]
            pub fn a() {}
            #[tynd_host_macros::skip]
            pub fn b() {}
            #[skip]
            pub fn c() {}
            #[other::skip]
            pub fn d() {}
            #[doc = "kept"]
            pub fn e() {}
        }
    })
    .unwrap();
    let names: Vec<_> = p.handlers.iter().map(|h| h.wire_name.as_str()).collect();
    assert_eq!(
        names,
        ["d", "e"],
        "only the recognised skip paths hide a method"
    );
}

#[test]
fn skipped_methods_bypass_validation() {
    // A `#[tynd::skip]` helper may use shapes the dispatcher cannot expose.
    let p = plan(parse_quote! {
        impl Api {
            #[tynd::skip]
            pub async fn helper(&mut self, (a, b): (u8, u8)) {}
        }
    })
    .unwrap();
    assert!(p.handlers.is_empty());
}

#[test]
fn rejects_duplicate_wire_names_after_camel_case() {
    let e = err(parse_quote! {
        impl Api {
            pub fn read_text() {}
            pub fn readText() {}
        }
    });
    assert!(e.contains("duplicate Tynd wire method 'readText'"), "{e}");
}

#[test]
fn rejects_unsupported_receivers() {
    for item in [
        parse_quote!(impl Api { pub fn f(&mut self) {} }),
        parse_quote!(impl Api { pub fn f(self) {} }),
        parse_quote!(impl Api { pub fn f(mut self) {} }),
        parse_quote!(impl Api { pub fn f(self: Box<Self>) {} }),
    ] {
        let e = err(item);
        assert!(e.contains("only supports `&self` or no receiver"), "{e}");
    }
}

#[test]
fn rejects_unsupported_signatures() {
    let cases: [(ItemImpl, &str); 7] = [
        (
            parse_quote!(impl Api { pub async fn f() {} }),
            "async methods are unsupported",
        ),
        (
            parse_quote!(impl Api { pub unsafe fn f() {} }),
            "unsafe methods",
        ),
        (
            parse_quote!(impl Api { pub const fn f() {} }),
            "generic methods are unsupported",
        ),
        (
            parse_quote!(impl Api { pub extern "C" fn f() {} }),
            "generic methods are unsupported",
        ),
        (
            parse_quote!(impl Api { pub fn f<T>(t: T) {} }),
            "generic methods are unsupported",
        ),
        (
            parse_quote!(impl Api { pub fn f(t: u8) where u8: Copy {} }),
            "generic methods are unsupported",
        ),
        (
            parse_quote!(impl Api { pub fn f((a, b): (u8, u8)) {} }),
            "plain identifier parameters",
        ),
    ];
    for (item, needle) in cases {
        let e = err(item);
        assert!(e.contains(needle), "{needle}: {e}");
    }
}

#[test]
fn is_skip_attr_matches_exact_paths_only() {
    let yes: [Attribute; 3] = [
        parse_quote!(#[tynd::skip]),
        parse_quote!(#[tynd_host_macros::skip]),
        parse_quote!(#[skip]),
    ];
    let no: [Attribute; 3] = [
        parse_quote!(#[tynd::skip::more]),
        parse_quote!(#[skipped]),
        parse_quote!(#[x::tynd::skip]),
    ];
    assert!(yes.iter().all(is_skip_attr));
    assert!(!no.iter().any(is_skip_attr));
}
