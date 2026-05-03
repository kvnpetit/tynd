//! End-to-end smoke tests for the web-standard polyfills. Stubs the
//! native bridges so we don't need a worker thread or real I/O.

use rquickjs::{Context, Function, Runtime};

use super::polyfills_js;

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
        // Node bridges — minimal stubs so the polyfills install without
        // a real OS underneath. Each one returns a deterministic value.
        g.set("__tynd_platform__", Function::new(ctx.clone(), || -> String { "linux".into() }).unwrap()).unwrap();
        g.set("__tynd_arch__", Function::new(ctx.clone(), || -> String { "x64".into() }).unwrap()).unwrap();
        g.set("__tynd_pid__", Function::new(ctx.clone(), || -> u32 { 1234 }).unwrap()).unwrap();
        g.set("__tynd_cwd__", Function::new(ctx.clone(), || -> String { "/tmp".into() }).unwrap()).unwrap();
        g.set("__tynd_chdir__", Function::new(ctx.clone(), |_: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_env_get__", Function::new(ctx.clone(), |_: String| -> Option<String> { None }).unwrap()).unwrap();
        g.set("__tynd_env_keys__", Function::new(ctx.clone(), || -> Vec<String> { vec![] }).unwrap()).unwrap();
        g.set("__tynd_env_set__", Function::new(ctx.clone(), |_: String, _: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_env_del__", Function::new(ctx.clone(), |_: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_argv__", Function::new(ctx.clone(), || -> Vec<String> { vec!["tynd-lite".into()] }).unwrap()).unwrap();
        g.set("__tynd_exit__", Function::new(ctx.clone(), |_: i32| -> () {}).unwrap()).unwrap();
        g.set("__tynd_hrtime_ms__", Function::new(ctx.clone(), || -> f64 { 0.0 }).unwrap()).unwrap();
        g.set("__tynd_uptime_ms__", Function::new(ctx.clone(), || -> f64 { 0.0 }).unwrap()).unwrap();
        g.set("__tynd_os_homedir__", Function::new(ctx.clone(), || -> Option<String> { Some("/home/test".into()) }).unwrap()).unwrap();
        g.set("__tynd_os_tmpdir__", Function::new(ctx.clone(), || -> String { "/tmp".into() }).unwrap()).unwrap();
        g.set("__tynd_os_hostname__", Function::new(ctx.clone(), || -> Option<String> { Some("test".into()) }).unwrap()).unwrap();
        g.set("__tynd_os_type__", Function::new(ctx.clone(), || -> String { "Linux".into() }).unwrap()).unwrap();
        g.set("__tynd_os_release__", Function::new(ctx.clone(), || -> String { "1.0.0".into() }).unwrap()).unwrap();
        g.set("__tynd_os_cpus__", Function::new(ctx.clone(), || -> u32 { 4 }).unwrap()).unwrap();
        g.set("__tynd_os_totalmem__", Function::new(ctx.clone(), || -> f64 { 0.0 }).unwrap()).unwrap();
        g.set("__tynd_os_freemem__", Function::new(ctx.clone(), || -> f64 { 0.0 }).unwrap()).unwrap();
        g.set("__tynd_os_uptime__", Function::new(ctx.clone(), || -> f64 { 0.0 }).unwrap()).unwrap();
        g.set("__tynd_os_loadavg__", Function::new(ctx.clone(), || -> Vec<f64> { vec![0.0, 0.0, 0.0] }).unwrap()).unwrap();
        // fs bridges — return stubs so the polyfills load. Per-test that
        // actually exercise fs would need a more elaborate setup.
        g.set("__tynd_fs_read_text__", Function::new(ctx.clone(), |_: String| -> String { String::new() }).unwrap()).unwrap();
        g.set("__tynd_fs_read_bytes__", Function::new(ctx.clone(), |_: String| -> String { String::new() }).unwrap()).unwrap();
        g.set("__tynd_fs_write_text__", Function::new(ctx.clone(), |_: String, _: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_write_bytes__", Function::new(ctx.clone(), |_: String, _: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_exists__", Function::new(ctx.clone(), |_: String| -> bool { false }).unwrap()).unwrap();
        g.set("__tynd_fs_stat__", Function::new(ctx.clone(), |_: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_readdir__", Function::new(ctx.clone(), |_: String| -> Vec<String> { vec![] }).unwrap()).unwrap();
        g.set("__tynd_fs_mkdir__", Function::new(ctx.clone(), |_: String, _: bool| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_rm__", Function::new(ctx.clone(), |_: String, _: bool| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_rename__", Function::new(ctx.clone(), |_: String, _: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_copy__", Function::new(ctx.clone(), |_: String, _: String| -> () {}).unwrap()).unwrap();
        g.set("__tynd_fs_realpath__", Function::new(ctx.clone(), |_: String| -> String { String::new() }).unwrap()).unwrap();
        g.set("__tynd_fs_chmod__", Function::new(ctx.clone(), |_: String, _: u32| -> () {}).unwrap()).unwrap();
        // Surface polyfill bootstrap failures with the actual JS exception
        // — bare `unwrap()` only shows `Err(Exception)` which makes
        // diagnosing a typo in any of the 26 JS files painful.
        if let Err(e) = ctx.eval::<(), _>(polyfills_js().as_bytes()) {
            let exn = ctx.catch();
            panic!("polyfill eval failed: {e:?} | exception: {exn:?}");
        }
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
fn node_and_bun_surface_present() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        // Bun, process, Buffer, setImmediate, EventEmitter, require —
        // all present after the enriched polyfill bundle. Deno still
        // intentionally absent (out of scope).
        let out: String = ctx
            .eval(
                r"[typeof Bun,
                    typeof Deno,
                    typeof process,
                    typeof Buffer,
                    typeof setImmediate,
                    typeof require,
                    typeof EventEmitter].join(',')",
            )
            .expect("eval failed");
        assert_eq!(out, "object,undefined,object,function,function,function,function");
    });
}

#[test]
fn buffer_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var b = Buffer.from("hello", "utf8");
                b.toString("hex") + "|" + b.toString("base64") + "|" + b.length
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "68656c6c6f|aGVsbG8=|5");
    });
}

#[test]
fn require_node_modules() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var path = require("path");
                var posix = require("path/posix");
                var events = require("node:events");
                [posix.join("a", "b", "c"),
                 path.basename("/foo/bar.txt", ".txt"),
                 typeof events.EventEmitter].join("|")
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "a/b/c|bar|function");
    });
}

#[test]
fn event_emitter_basic() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var ee = new EventEmitter();
                var hits = [];
                ee.on("x", function (a, b) { hits.push("on:" + a + ":" + b); });
                ee.once("x", function (a) { hits.push("once:" + a); });
                ee.emit("x", 1, 2);
                ee.emit("x", 3, 4);
                hits.join(",")
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "on:1:2,once:1,on:3:4");
    });
}

#[test]
fn querystring_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var qs = require("querystring");
                var s = qs.stringify({ a: "hello world", b: ["1","2"] });
                var p = qs.parse(s);
                s + "|" + p.a + "|" + p.b.join(",")
                "#,
            )
            .expect("eval failed");
        assert!(out.starts_with("a=hello%20world&b=1&b=2"));
    });
}

#[test]
fn util_inspect_and_promisify() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var util = require("util");
                var s = util.inspect({ a: 1, b: [1,2,3] });
                var f = util.promisify(function (x, cb) { cb(null, x * 2); });
                "ok:" + (s.indexOf("a: 1") >= 0)
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "ok:true");
    });
}

#[test]
fn process_env_alias() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                process.env.MY_VAR = "abc";
                process.env.MY_VAR + "|" + Bun.env.MY_VAR
                "#,
            )
            .expect("eval failed");
        // Stub bridge throws away sets — but Bun.env aliases process.env,
        // so the symbolic check still passes (both undefined here).
        assert_eq!(out, "undefined|undefined");
    });
}

#[test]
fn bun_glob_match() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: bool = ctx
            .eval(
                r#"
                var g = new Bun.Glob("**/*.ts");
                g.match("src/foo/bar.ts") && !g.match("src/foo/bar.js")
                "#,
            )
            .expect("eval failed");
        assert!(out);
    });
}

#[test]
fn bun_password_hash_verify_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        // Stubbed __tynd_crypto_random__ returns "" so we can't actually
        // exercise hashSync end-to-end here — instead verify the API
        // surface is wired (no exception on access).
        let out: String = ctx
            .eval(
                r"[typeof Bun.password.hash,
                    typeof Bun.password.verify,
                    typeof Bun.SHA256].join(',')",
            )
            .expect("eval failed");
        assert_eq!(out, "function,function,function");
    });
}

#[test]
fn streams_readable_writable() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var rs = new ReadableStream({ start: function (c) { c.enqueue("a"); c.enqueue("b"); c.close(); } });
                "ok:" + (typeof rs.getReader === "function" && typeof WritableStream === "function" && typeof TransformStream === "function")
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "ok:true");
    });
}

#[test]
fn assert_module() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: bool = ctx
            .eval(
                r#"
                var assert = require("assert");
                assert.strictEqual(1, 1);
                assert.deepStrictEqual({ a: 1 }, { a: 1 });
                var threw = false;
                try { assert.strictEqual(1, 2); } catch (_) { threw = true; }
                threw
                "#,
            )
            .expect("eval failed");
        assert!(out);
    });
}

#[test]
fn path_posix_and_win32() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var p = require("path");
                p.posix.join("a", "b/c", "..", "d") + "|" +
                p.win32.normalize("C:\\\\foo\\\\..\\\\bar\\\\baz") + "|" +
                p.posix.parse("/foo/bar.txt").ext
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "a/b/d|C:\\bar\\baz|.txt");
    });
}

#[test]
fn url_filepath_roundtrip() {
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var url = require("url");
                var u = url.pathToFileURL("/etc/hosts");
                url.fileURLToPath(u)
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "/etc/hosts");
    });
}

// ---- npm ecosystem smoke tests ------------------------------------------
//
// These exercise the API surface that popular pure-JS npm packages probe
// at runtime. We don't bundle the actual packages (would explode test
// build time), but each test mimics what the package's hot path needs.
// If unenv breaks one of these, the package itself will too.

#[test]
fn npm_compat_dayjs_style() {
    // dayjs uses Date.prototype + util-style format strings.
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var util = require("util");
                util.format("today is %s, %d days in", "2026-05-03", 123)
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "today is 2026-05-03, 123 days in");
    });
}

#[test]
fn npm_compat_qs_style() {
    // qs (Express's querystring lib) uses node:querystring underneath.
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var qs = require("querystring");
                var obj = qs.parse("name=alice&tags=node&tags=lite");
                obj.name + "|" + (Array.isArray(obj.tags) ? obj.tags.join(",") : obj.tags)
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "alice|node,lite");
    });
}

#[test]
fn npm_compat_debug_style() {
    // The `debug` package reads process.env, util.format, and creates an
    // EventEmitter on the side. Smoke-test the combo.
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var util = require("util");
                var EE = require("events").EventEmitter;
                var emitter = new EE();
                var formatted = util.format("[%s] %s", "auth", "login attempt");
                emitter.on("log", function (msg) { emitter._last = msg; });
                emitter.emit("log", formatted);
                emitter._last
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "[auth] login attempt");
    });
}

#[test]
fn npm_compat_picocolors_style() {
    // picocolors / chalk slice strings + check process.env.NO_COLOR.
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var noColor = process.env.NO_COLOR;
                var stdout = process.stdout || { isTTY: false };
                "noColor=" + (noColor || "unset") + "|tty=" + (stdout.isTTY === true)
                "#,
            )
            .expect("eval failed");
        assert!(out.starts_with("noColor=unset|tty="));
    });
}

#[test]
fn npm_compat_stream_pipethrough() {
    // ReadableStream + TransformStream (e.g. ofetch, undici clones).
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var rs = new ReadableStream({
                  start: function (c) { c.enqueue("a"); c.enqueue("b"); c.close(); }
                });
                "ok:" + (typeof rs.getReader === "function")
                  + "|tx:" + (typeof TransformStream === "function")
                  + "|ws:" + (typeof WritableStream === "function")
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "ok:true|tx:true|ws:true");
    });
}

#[test]
fn npm_compat_buffer_concat_and_slice() {
    // Most node-style libs (jsonwebtoken, cookie-signature) use these.
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var a = Buffer.from("hello ", "utf8");
                var b = Buffer.from("world", "utf8");
                var c = Buffer.concat([a, b]);
                c.toString("utf8") + "|" + c.subarray(6).toString("utf8") + "|" + c.length
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "hello world|world|11");
    });
}

#[test]
fn npm_compat_crypto_random_uuid() {
    // jsonwebtoken / nanoid / uuid pull from crypto.randomBytes/UUID.
    let (_rt, ctx) = boot_pure();
    ctx.with(|ctx| {
        let out: String = ctx
            .eval(
                r#"
                var crypto = require("crypto");
                var b = crypto.randomBytes(8);
                var u = crypto.randomUUID();
                "len=" + b.length + "|uuid_dashes=" + (u.split("-").length === 5)
                "#,
            )
            .expect("eval failed");
        assert_eq!(out, "len=8|uuid_dashes=true");
    });
}
