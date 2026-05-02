//! Microbenchmark: rkyv 0.8 vs serde_json on the IPC hot path.
//!
//! Drives the data-driven decision on whether to migrate the host↔plugin
//! and host↔Bun edges to a binary archive format. The JS edge stays on
//! JSON (devalue) regardless — V8 parses JSON natively in C++ and rkyv
//! has no JS runtime.
//!
//! What we measure:
//!   1. Serialize    — Rust struct  → bytes
//!   2. Deserialize  — bytes        → owned Rust struct
//!   3. Field access — bytes        → read one field (rkyv = pointer cast,
//!                                    serde_json = full parse first)
//!   4. Streaming    — 10 000 RpcResponses serialized in a single batch,
//!                     mirroring the README's "10k+ yields/sec" claim.
//!
//! Three payload sizes per scenario: small ~64 B, medium ~4 KB, large
//! ~64 KB. These bracket the realistic IPC traffic shape — chatty RPCs
//! (input events, control plane), document-sized responses (LLM token
//! chunks, log lines), and bulky transfers (file slices, screenshot
//! tiles).
//!
//! Run:
//!     cargo bench -p tynd-host --bench ipc_rkyv_vs_serde
//!
//! Quick smoke (skip the full Criterion warm-up):
//!     cargo bench -p tynd-host --bench ipc_rkyv_vs_serde -- \
//!         --sample-size 10 --measurement-time 2
//!
//! `--quick` is criterion 0.6+; we're on 0.5 so we use the manual flags.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rkyv::{rancor::Error as RkyvError, Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

// ---------------------------------------------------------------------------
// Data shapes — mirror an `os_call` RPC and its streamed yield response.
// Both serde and rkyv derive on the same types so the bench compares
// formats, not type definitions.
// ---------------------------------------------------------------------------

#[derive(SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone)]
struct RpcCall {
    id: u64,
    method: String,
    args: Vec<RpcArg>,
    timestamp: u64,
}

#[derive(SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone)]
enum RpcArg {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
}

/// Streaming yield frame — what `eval_yield_batch` coalesces. We bench
/// the wire-encode cost on the Rust side; the JS side cost (decode +
/// `evaluate_script` round trip) is out of scope here.
#[derive(SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone)]
struct RpcResponse {
    id: u64,
    seq: u32,
    payload: String,
}

// ---------------------------------------------------------------------------
// Payload builders — produce the three size classes the bench iterates over.
// ---------------------------------------------------------------------------

fn payload_small() -> RpcCall {
    // ~64 B JSON: short method + 1 short string arg. Mirrors a
    // control-plane call like `window.setTitle("hi")`.
    RpcCall {
        id: 0x0193_3a4f_4c3a_7c6f,
        method: "fs.readText".to_string(),
        args: vec![RpcArg::Str("a.txt".to_string())],
        timestamp: 1_700_000_000_000,
    }
}

fn payload_medium() -> RpcCall {
    // ~4 KB: 1 string arg ~3.8 KB. Document-sized, like an LLM chunk
    // or an inlined config blob.
    let body = "x".repeat(3_900);
    RpcCall {
        id: 0x0193_3a4f_4c3a_7c70,
        method: "fs.writeText".to_string(),
        args: vec![RpcArg::Str(body)],
        timestamp: 1_700_000_000_001,
    }
}

fn payload_large() -> RpcCall {
    // ~64 KB Bytes arg. Bulky transfer — file slice, screenshot tile.
    let bytes = vec![0xABu8; 64 * 1024];
    RpcCall {
        id: 0x0193_3a4f_4c3a_7c71,
        method: "fs.writeBinary".to_string(),
        args: vec![RpcArg::Bytes(bytes)],
        timestamp: 1_700_000_000_002,
    }
}

fn sizes() -> [(&'static str, RpcCall); 3] {
    [
        ("small_64B", payload_small()),
        ("medium_4KB", payload_medium()),
        ("large_64KB", payload_large()),
    ]
}

// ---------------------------------------------------------------------------
// Bench 1 — serialize.
// ---------------------------------------------------------------------------

fn bench_serialize(c: &mut Criterion) {
    let mut g = c.benchmark_group("ipc/serialize");
    for (label, payload) in sizes() {
        // Throughput in bytes lets criterion print MB/s alongside ns/op,
        // which is the metric that actually matters when comparing
        // payload-size scaling.
        let json_size = serde_json::to_vec(&payload).unwrap().len() as u64;
        g.throughput(Throughput::Bytes(json_size));

        g.bench_with_input(BenchmarkId::new("serde_json", label), &payload, |b, p| {
            b.iter(|| {
                let v = serde_json::to_vec(black_box(p)).unwrap();
                black_box(v);
            });
        });

        g.bench_with_input(BenchmarkId::new("rkyv", label), &payload, |b, p| {
            b.iter(|| {
                let v = rkyv::to_bytes::<RkyvError>(black_box(p)).unwrap();
                black_box(v);
            });
        });
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Bench 2 — deserialize (full owned struct, both formats).
// ---------------------------------------------------------------------------

fn bench_deserialize_full(c: &mut Criterion) {
    let mut g = c.benchmark_group("ipc/deserialize_full");
    for (label, payload) in sizes() {
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let rkyv_bytes = rkyv::to_bytes::<RkyvError>(&payload).unwrap();
        g.throughput(Throughput::Bytes(json_bytes.len() as u64));

        g.bench_with_input(
            BenchmarkId::new("serde_json", label),
            &json_bytes,
            |b, bytes| {
                b.iter(|| {
                    let v: RpcCall = serde_json::from_slice(black_box(bytes)).unwrap();
                    black_box(v);
                });
            },
        );

        g.bench_with_input(
            BenchmarkId::new("rkyv", label),
            &rkyv_bytes,
            |b, bytes| {
                b.iter(|| {
                    // High-level safe API: validates with bytecheck then
                    // deserializes into an owned `RpcCall`. This is the
                    // apples-to-apples comparison for serde_json's
                    // `from_slice` — both end with an owned struct.
                    let v: RpcCall = rkyv::from_bytes::<RpcCall, RkyvError>(black_box(bytes)).unwrap();
                    black_box(v);
                });
            },
        );
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Bench 3 — field access (read one field without owning the rest).
// rkyv pays a pointer offset; serde_json must parse the whole payload.
// This is where rkyv's "zero-copy" claim cashes in.
// ---------------------------------------------------------------------------

fn bench_field_access(c: &mut Criterion) {
    let mut g = c.benchmark_group("ipc/field_access");
    for (label, payload) in sizes() {
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let rkyv_bytes = rkyv::to_bytes::<RkyvError>(&payload).unwrap();
        g.throughput(Throughput::Bytes(json_bytes.len() as u64));

        g.bench_with_input(
            BenchmarkId::new("serde_json", label),
            &json_bytes,
            |b, bytes| {
                // Worst-case-realistic: we don't have a streaming JSON
                // parser plumbed through, so reading `id` means parsing
                // to `Value` and extracting. This mirrors the actual cost
                // the dispatcher would pay today.
                b.iter(|| {
                    let v: serde_json::Value = serde_json::from_slice(black_box(bytes)).unwrap();
                    let id = v.get("id").and_then(serde_json::Value::as_u64).unwrap();
                    black_box(id);
                });
            },
        );

        g.bench_with_input(
            BenchmarkId::new("rkyv", label),
            &rkyv_bytes,
            |b, bytes| {
                b.iter(|| {
                    // `access` validates layout (bytecheck) once then
                    // hands back a `&ArchivedRpcCall`. Field reads are
                    // pointer derefs — no parse, no alloc.
                    let archived = rkyv::access::<ArchivedRpcCall, RkyvError>(black_box(bytes)).unwrap();
                    let id: u64 = archived.id.into();
                    black_box(id);
                });
            },
        );
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Bench 4 — streaming throughput. Serialize 10k RpcResponses to wire
// bytes and measure ops/sec. Backs the README's "10k+ yields/sec"
// claim against both formats.
//
// We pay the per-frame serialize cost only — no `evaluate_script`
// round trip, no JS-side decode. That isolates the format cost from
// the IPC plumbing cost.
// ---------------------------------------------------------------------------

fn bench_streaming_yield(c: &mut Criterion) {
    const N: usize = 10_000;
    // ~64 B payload per yield — typical LLM-token / log-tail chunk.
    let frames: Vec<RpcResponse> = (0..N as u32)
        .map(|i| RpcResponse {
            id: 0x0193_3a4f_4c3a_7c6f,
            seq: i,
            payload: "token".repeat(8), // ~40 B
        })
        .collect();

    let mut g = c.benchmark_group("ipc/streaming_10k_yields");
    g.throughput(Throughput::Elements(N as u64));

    g.bench_function("serde_json", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for f in &frames {
                let v = serde_json::to_vec(black_box(f)).unwrap();
                total += v.len();
                black_box(v);
            }
            black_box(total);
        });
    });

    g.bench_function("rkyv", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for f in &frames {
                let v = rkyv::to_bytes::<RkyvError>(black_box(f)).unwrap();
                total += v.len();
                black_box(v);
            }
            black_box(total);
        });
    });

    g.finish();
}

criterion_group!(
    benches,
    bench_serialize,
    bench_deserialize_full,
    bench_field_access,
    bench_streaming_yield,
);
criterion_main!(benches);
