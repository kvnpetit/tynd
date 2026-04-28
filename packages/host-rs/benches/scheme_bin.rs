//! Microbenchmark for the `tynd-bin://` scheme handler.
//!
//! `scheme_bin::handle_async` is the entry point for the binary scheme
//! and the only public function in the module — but it takes a
//! `wry::RequestAsyncResponder` which has no public constructor (wry
//! mints them inside its custom-protocol callback machinery). We can't
//! build one in a bench without re-exporting wry internals or wrapping
//! the dispatcher.
//!
//! What we *can* benchmark is the request shape on the way in. The
//! cost of building the `wry::http::Request` and exercising its URI /
//! header parsing is a meaningful proxy for the per-call overhead the
//! scheme handler pays before the work hands off to `call_pool`. If
//! that climbs the whole binary scheme gets slower regardless of the
//! actual route's cost.
//!
//! When `scheme_bin::dispatch` (or `parse_query`) becomes `pub(crate)`
//! and we expose a bench-only re-export, swap this bench to call into
//! the real dispatcher.
//!
//! Baseline (fill in after first `cargo bench` run):
//!   scheme_bin/request_build  ~ ?? ns
//!
//! Run with:
//!   cargo bench -p tynd-host --bench scheme_bin

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use wry::http::Request;

fn bench_request_build(c: &mut Criterion) {
    let url = "tynd-bin://localhost/fs/readBinary?path=C%3A%2Ffoo%2Fbar%2Ffile.bin&createDirs=1";
    c.bench_function("scheme_bin/request_build", |b| {
        b.iter(|| {
            let req = Request::builder()
                .uri(black_box(url))
                .method("GET")
                .body(Vec::<u8>::new())
                .unwrap();
            // Force materialization of uri.path() and uri.query() so
            // the bench reflects what `dispatch` actually consumes —
            // `black_box` on the values stops constant-folding.
            black_box(req.uri().path());
            black_box(req.uri().query().unwrap_or(""));
            black_box(req);
        });
    });
}

criterion_group!(benches, bench_request_build);
criterion_main!(benches);
