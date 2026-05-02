//! Microbenchmark for the `tynd-bin://` scheme handler.
//!
//! The real entry point is `scheme_bin::handle_async`, which takes a
//! `wry::RequestAsyncResponder`. That type has no public constructor —
//! wry mints them inside its custom-protocol callback machinery — so we
//! can't drive `handle_async` from a bench without re-exporting wry
//! internals or wrapping the dispatcher behind a `pub(crate)` shim.
//!
//! What we *can* benchmark is everything the dispatcher does *before*
//! the route fan-out: build the `wry::http::Request`, materialize
//! `uri.path()` and `uri.query()`, and parse the query string into the
//! same `HashMap<String, String>` shape `dispatch` consumes (via
//! `form_urlencoded::parse`, which is what the real handler uses too —
//! see `scheme_bin::parse_query`). That mirrors the per-call overhead
//! every binary route pays before the actual work hands off to
//! `os::call_pool`. If this climbs the whole binary scheme gets slower
//! regardless of the actual route's cost.
//!
//! When `scheme_bin::dispatch` (or `parse_query`) becomes `pub(crate)`
//! and we expose a bench-only re-export, swap this bench to call into
//! the real dispatcher. Until then, the parser bench below is the
//! tightest available proxy — it exercises the same crate
//! (`form_urlencoded`) with the same allocations the real path does.
//!
//! Baseline (target `release`, criterion sample-size=10
//! measurement-time=2. `request_build` recorded from a real run on
//! Windows 11 / AMD x86_64, 2026-05-02. The `parse_query` +
//! `full_pipeline` figures are analytic estimates from
//! `form_urlencoded::parse` complexity — CI on `ubuntu-latest` is the
//! source of truth for those, see `.github/workflows/bench.yml`):
//!   scheme_bin/request_build       ~ 271 ns   [254 ns ; 296 ns]
//!   scheme_bin/parse_query/short   ~ 410 ns   (3 keys, ~80 B query)
//!   scheme_bin/parse_query/long    ~ 1.1 µs   (10 keys, ~400 B query)
//!   scheme_bin/full_pipeline       ~ 750 ns   (build + path + query parse)
//!
//! `full_pipeline` is the closest proxy we have for the per-call
//! overhead `dispatch` charges before the route match. CI fails if any
//! sub-bench regresses by > 5 % vs the baseline measured on the same
//! runner — see `.github/workflows/bench.yml`.
//!
//! Run with:
//!   cargo bench -p tynd-host --bench scheme_bin

use std::collections::HashMap;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use wry::http::Request;

/// Build a `wry::http::Request<Vec<u8>>` shaped like a real
/// `fs.readBinary` call. Mirrors what wry hands the scheme callback at
/// the top of `dispatch`.
fn build_request(url: &str, body: Vec<u8>) -> Request<Vec<u8>> {
    Request::builder()
        .uri(url)
        .method("GET")
        .body(body)
        .unwrap()
}

/// Reimplements `scheme_bin::parse_query` byte-for-byte (same crate,
/// same percent-decode + `+`-as-space + UTF-8 validation, same
/// last-wins `HashMap` collect). When the real `parse_query` becomes
/// `pub(crate)` and is re-exported for benches, drop this and call into
/// the real one.
fn parse_query(q: &str) -> HashMap<String, String> {
    form_urlencoded::parse(q.as_bytes())
        .into_owned()
        .collect::<HashMap<_, _>>()
}

fn bench_request_build(c: &mut Criterion) {
    let url = "tynd-bin://localhost/fs/readBinary?path=C%3A%2Ffoo%2Fbar%2Ffile.bin&createDirs=1";
    c.bench_function("scheme_bin/request_build", |b| {
        b.iter(|| {
            let req = build_request(black_box(url), Vec::<u8>::new());
            // Force materialization of uri.path() and uri.query() so
            // the bench reflects what `dispatch` actually consumes —
            // `black_box` on the values stops constant-folding.
            black_box(req.uri().path());
            black_box(req.uri().query().unwrap_or(""));
            black_box(req);
        });
    });
}

fn bench_parse_query(c: &mut Criterion) {
    // Two size classes. `short` is the typical fs.readBinary call
    // (path + maybe createDirs). `long` is the worst-case shape the
    // route can see today — full path + algo + encoding + a handful of
    // hypothetical future flags.
    let cases = [
        (
            "short",
            "path=C%3A%2Ffoo%2Fbar%2Ffile.bin&createDirs=1&algo=sha256",
        ),
        (
            "long",
            "path=C%3A%2FUsers%2Falice%2FDocuments%2Fproject%2Fsubdir%2Ffile.bin&\
             createDirs=1&algo=blake3&encoding=hex&offset=0&length=4096&\
             flag1=value1&flag2=value2&flag3=value3&flag4=value4",
        ),
    ];

    let mut g = c.benchmark_group("scheme_bin/parse_query");
    for (label, query) in cases {
        g.bench_with_input(BenchmarkId::from_parameter(label), &query, |b, q| {
            b.iter(|| {
                let m = parse_query(black_box(q));
                black_box(m);
            });
        });
    }
    g.finish();
}

/// End-to-end "before-route-match" pipeline: this is the closest proxy
/// we can get to `dispatch`'s per-call overhead from outside the crate.
/// Builds the Request, splits the path on `/`, parses the query, and
/// looks up the `path` key from the resulting map (the first thing
/// every fs route does).
fn bench_full_pipeline(c: &mut Criterion) {
    let url = "tynd-bin://localhost/fs/readBinary?path=C%3A%2Ffoo%2Fbar%2Ffile.bin&createDirs=1";
    c.bench_function("scheme_bin/full_pipeline", |b| {
        b.iter(|| {
            let req = build_request(black_box(url), Vec::<u8>::new());
            let path = req.uri().path().trim_start_matches('/');
            let query = parse_query(req.uri().query().unwrap_or(""));
            // Mirror the route match's split + first-arg lookup — the
            // exact pre-work `route_read_binary` does inside dispatch.
            let split = path.split_once('/');
            let p = query.get("path").cloned();
            black_box(split);
            black_box(p);
            black_box(req);
        });
    });
}

criterion_group!(
    benches,
    bench_request_build,
    bench_parse_query,
    bench_full_pipeline
);
criterion_main!(benches);
