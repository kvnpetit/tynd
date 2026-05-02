//! Microbenchmark for the streaming-batch eval string builder.
//!
//! `eval_yield_batch` is the hot path on bursty streams (LLM token
//! decoding, log tailing, etc.). It coalesces N pending yields into a
//! single `evaluate_script` call to amortize the wry/JS-bridge round
//! trip. The cited "~30x main-thread cost reduction" in the source
//! comment depends on this builder staying cheap — if rebuilding the
//! batch string itself costs more than the round trips it saves we lose
//! the win.
//!
//! Baseline (recorded 2026-05-02 on Windows 11, AMD x86_64, target
//! `release`, criterion sample-size=10 measurement-time=2):
//!   eval_yield_batch/100x256B  ~ 41 µs   [39.4 µs ; 42.4 µs]
//!
//! That's ~410 ns per (id, payload) pair amortized — well under the
//! `evaluate_script` round-trip cost the batch is replacing
//! (~30 µs per call on wry/JS bridge), so the coalescer keeps paying
//! its keep at this size. CI fails if the batch builder regresses by
//! > 5 % vs the baseline measured on the same runner — see
//! `.github/workflows/bench.yml`.
//!
//! Run with:
//!   cargo bench -p tynd-host --bench ipc_yield_batch

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use tynd_host::ipc::eval_yield_batch;

/// Build a realistic batch: 100 chunks, ~256 B per chunk = ~25 KB
/// payload. Larger batches hit our `STREAM_CREDIT=64` ceiling on the
/// JS side; 100 here gives headroom to measure scaling without
/// representing a pathological case.
fn batch_100_chunks() -> Vec<(String, String)> {
    let mut v = Vec::with_capacity(100);
    for i in 0..100 {
        let id = format!("01933a4f-4c3a-7c6f-b5d2-{i:012}");
        let mut chunk = String::with_capacity(256);
        chunk.push_str(r#"["chunk-payload","#);
        while chunk.len() < 256 {
            chunk.push_str(r#""token-data","#);
        }
        chunk.push(']');
        v.push((id, chunk));
    }
    v
}

fn bench_yield_batch(c: &mut Criterion) {
    let pairs = batch_100_chunks();
    c.bench_function("eval_yield_batch/100x256B", |b| {
        b.iter(|| {
            let s = eval_yield_batch(black_box(&pairs));
            black_box(s);
        });
    });
}

criterion_group!(benches, bench_yield_batch);
criterion_main!(benches);
