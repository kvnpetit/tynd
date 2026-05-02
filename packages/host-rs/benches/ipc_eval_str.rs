//! Microbenchmarks for the JSON-eval string builders in `ipc.rs`.
//!
//! These functions sit on the *very* hot path: every single backend
//! return, every yielded chunk, every backend-emitted event lands in
//! one of them on its way to `webview.evaluate_script(...)`. A few hundred
//! nanoseconds of regression here multiplies fast on bursty streams.
//!
//! Baseline (recorded 2026-05-02 on Windows 11, AMD x86_64, target
//! `release`, criterion sample-size=10 measurement-time=2 — same flags
//! the CI workflow uses, see `.github/workflows/bench.yml`):
//!   eval_resolve/1KB      ~ 1.48 µs   [1.39  µs ;  1.70  µs]
//!   eval_yield/1KB        ~ 1.48 µs   [1.35  µs ;  1.63  µs]
//!   eval_dispatch/1KB     ~ 1.45 µs   [1.29  µs ;  1.65  µs]
//!
//! Numbers will differ on CI hardware (GitHub `ubuntu-latest`); the
//! workflow re-runs on `main` and the PR HEAD then computes the delta
//! with `critcmp`. CI fails if any bench regresses by > 5 % vs the
//! baseline measured on the same runner.
//!
//! Run with:
//!   cargo bench -p tynd-host --bench ipc_eval_str
//!
//! Compile-only (CI sanity check):
//!   cargo bench -p tynd-host --no-run

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tynd_host::ipc::{eval_dispatch, eval_resolve, eval_yield};

/// Build a ~1 KB devalue-shaped wire payload string. Mirrors what the
/// full runtime actually emits (devalue arrays of mixed types) so the
/// bench costs reflect production load, not a synthetic best-case.
fn payload_1kb() -> String {
    // devalue stringify of a record with strings, numbers, dates, etc.
    // The exact shape doesn't matter — only the size class does.
    let mut s = String::with_capacity(1024);
    s.push_str(r#"[["k","v","arr","n","ts","map"],"#);
    s.push_str(r#""hello world",42,"#);
    s.push_str(r#"["a","b","c","d","e","f","g","h","i","j"],"#);
    while s.len() < 1024 {
        s.push_str(r#""filler-segment-to-reach-1k-bytes","#);
    }
    s.push(']');
    s
}

fn bench_eval_resolve(c: &mut Criterion) {
    let payload = payload_1kb();
    let id = "01933a4f-4c3a-7c6f-b5d2-1a2b3c4d5e6f";
    c.bench_function("eval_resolve/1KB", |b| {
        b.iter(|| {
            // black_box on inputs prevents constant-folding; on the
            // output it stops LLVM from eliding the formatted string.
            let s = eval_resolve(black_box(id), black_box(true), black_box(&payload));
            black_box(s);
        });
    });
}

fn bench_eval_yield(c: &mut Criterion) {
    let payload = payload_1kb();
    let id = "01933a4f-4c3a-7c6f-b5d2-1a2b3c4d5e6f";
    c.bench_function("eval_yield/1KB", |b| {
        b.iter(|| {
            let s = eval_yield(black_box(id), black_box(&payload));
            black_box(s);
        });
    });
}

fn bench_eval_dispatch(c: &mut Criterion) {
    let payload = payload_1kb();
    let name = "user:posted";
    c.bench_function("eval_dispatch/1KB", |b| {
        b.iter(|| {
            let s = eval_dispatch(black_box(name), black_box(&payload));
            black_box(s);
        });
    });
}

criterion_group!(
    benches,
    bench_eval_resolve,
    bench_eval_yield,
    bench_eval_dispatch
);
criterion_main!(benches);
