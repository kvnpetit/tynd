//! End-to-end smoke tests for the web-standard polyfills. Stubs the
//! native bridges so we don't need a worker thread or real I/O.

use rquickjs::{Context, Function, Runtime};

use super::POLYFILLS_JS;

fn boot_pure() -> (Runtime, Context) {
    let rt = Runtime::new().unwrap();
    let ctx = Context::full(&rt).unwrap();
    ctx.with(|ctx| {
        let g = ctx.globals();
        g.set(
            "__tynd_crypto_random__",
            Function::new(ctx.clone(), |_: u32| -> String { String::new() }).unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_crypto_digest__",
            Function::new(ctx.clone(), |_: String, _: String| -> String {
                String::new()
            })
            .unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_crypto_hmac_sign__",
            Function::new(ctx.clone(), |_: String, _: String, _: String| -> String {
                String::new()
            })
            .unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_crypto_hmac_verify__",
            Function::new(
                ctx.clone(),
                |_: String, _: String, _: String, _: String| -> bool { false },
            )
            .unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_fetch_start__",
            Function::new(ctx.clone(), |_: String, _: String, _: String| {}).unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_fetch_abort__",
            Function::new(ctx.clone(), |_: String| {}).unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_ws_connect__",
            Function::new(ctx.clone(), |_: String, _: String, _: String| {}).unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_ws_send__",
            Function::new(ctx.clone(), |_: String, _: String, _: bool| {}).unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_ws_close__",
            Function::new(ctx.clone(), |_: String, _: u16, _: String| {}).unwrap(),
        )
        .unwrap();
        g.set(
            "__tynd_perf_now__",
            Function::new(ctx.clone(), || -> f64 { 0.0 }).unwrap(),
        )
        .unwrap();
        ctx.eval::<(), _>(POLYFILLS_JS.as_bytes()).unwrap();
    });
    (rt, ctx)
}

#[test]
fn atob_btoa_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(r#"atob(btoa("hello world"))"#)
            .expect("eval failed");
        assert_eq!(out, "hello world");
    });
}

#[test]
fn text_encoder_decoder_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var e = new TextEncoder();
                var d = new TextDecoder();
                d.decode(e.encode("café ☕ 🐙"))
            "#,
            )
            .expect("eval failed");
        assert_eq!(out, "café ☕ 🐙");
    });
}

#[test]
fn abort_controller_signals() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: bool = ctx
            .eval(
                r#"
                var c = new AbortController();
                var fired = false;
                c.signal.addEventListener("abort", function(){ fired = true });
                c.abort();
                fired && c.signal.aborted
            "#,
            )
            .expect("eval failed");
        assert!(out);
    });
}

#[test]
fn url_parse_and_search_params() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var u = new URL("https://example.com:8080/path?x=1&y=2#frag");
                u.hostname + "|" + u.port + "|" + u.searchParams.get("y") + "|" + u.hash
            "#,
            )
            .expect("eval failed");
        assert_eq!(out, "example.com|8080|2|#frag");
    });
}

#[test]
fn url_search_params_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var p = new URLSearchParams();
                p.append("a", "hello world");
                p.append("b", "café");
                p.toString()
            "#,
            )
            .expect("eval failed");
        assert_eq!(out, "a=hello+world&b=caf%C3%A9");
    });
}

#[test]
fn structured_clone_deep_with_circular() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var src = { a: 1, d: new Date(0), m: new Map([["k", [1,2,3]]]) };
                src.self = src;
                var c = structuredClone(src);
                [c !== src, c.a, c.d.getTime(), c.m.get("k").length, c.self === c].join("|")
            "#,
            )
            .expect("eval failed");
        assert_eq!(out, "true|1|0|3|true");
    });
}

#[test]
fn promise_with_resolvers() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: bool = ctx
            .eval(
                r#"
                var d = Promise.withResolvers();
                typeof d.resolve === "function"
                  && typeof d.reject === "function"
                  && d.promise instanceof Promise
            "#,
            )
            .expect("eval failed");
        assert!(out);
    });
}

#[test]
fn blob_roundtrip_text() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        ctx.eval::<(), _>(
            br#"
                var b = new Blob(["hello ", "world"], { type: "text/plain" });
                globalThis._blobSize = b.size;
                globalThis._blobType = b.type;
            "#,
        )
        .expect("eval failed");
        let size: u32 = ctx.globals().get("_blobSize").unwrap();
        let ty: String = ctx.globals().get("_blobType").unwrap();
        assert_eq!(size, 11);
        assert_eq!(ty, "text/plain");
    });
}

#[test]
fn formdata_append_and_get() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var f = new FormData();
                f.append("x", "1");
                f.append("x", "2");
                f.set("y", "z");
                f.getAll("x").join(",") + "|" + f.get("y")
            "#,
            )
            .expect("eval failed");
        assert_eq!(out, "1,2|z");
    });
}

#[test]
fn fetch_headers_normalize_keys() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var h = new Headers({ "Content-Type": "text/plain" });
                h.append("X-Custom", "a");
                h.append("X-Custom", "b");
                h.get("content-type") + "|" + h.get("x-custom")
            "#,
            )
            .expect("eval failed");
        assert_eq!(out, "text/plain|a, b");
    });
}

#[test]
fn crypto_subtle_hmac_only() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r"[typeof crypto.subtle.digest,
                    typeof crypto.subtle.sign,
                    typeof crypto.subtle.verify,
                    typeof crypto.subtle.importKey,
                    typeof crypto.subtle.encrypt].join(',')",
            )
            .expect("eval failed");
        // encrypt is intentionally undefined on lite — use @noble/ciphers.
        assert_eq!(out, "function,function,function,function,undefined");
    });
}

#[test]
fn no_bun_or_node_surface() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r"[typeof Bun,
                    typeof Deno,
                    typeof process,
                    typeof Buffer,
                    typeof setImmediate].join(',')",
            )
            .expect("eval failed");
        assert_eq!(out, "undefined,undefined,undefined,undefined,undefined");
    });
}
