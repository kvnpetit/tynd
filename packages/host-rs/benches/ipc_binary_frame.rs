//! Microbenchmark: U1 Layer 1 — Bun↔host wire format selection.
//!
//! Data-driven evidence for picking CBOR over JSON on the Bun stdin/
//! stdout edge. We compare three encodings on the same `Call` frame
//! shape that `bun.rs::push_call_line` actually serializes:
//!
//!   1. `serde_json::to_vec`       — baseline, what ships today.
//!   2. `ciborium::ser::into_writer` — CBOR, the U1 Layer 1 binary path.
//!   3. `rkyv` zero-copy archive    — Layer 2 (deferred), as a ceiling
//!      reference so we know how much is left on the table after CBOR.
//!
//! Three payload sizes:
//!   - **small**  ~64 B body — control-plane RPC (window resize, ack).
//!   - **medium** ~4 KB body — typical user RPC return (sql row, dom event).
//!   - **large**  ~64 KB body — LLM token chunk batch / file slice.
//!
//! Run:
//!     cargo bench -p tynd-host --bench ipc_binary_frame
//!
//! Quick smoke (skip Criterion warm-up):
//!     cargo bench -p tynd-host --bench ipc_binary_frame -- \
//!         --sample-size 10 --measurement-time 2

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rkyv::{rancor::Error as RkyvError, Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};

// ---------------------------------------------------------------------------
// Frame shapes — mirror the `WireMsg::Call` shape in `tynd-full/src/bun.rs`.
// Both serde and rkyv derive on the same types so the bench compares
// formats, not type definitions.
// ---------------------------------------------------------------------------

#[derive(SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone)]
struct CallFrame {
    /// Discriminator field — `WireMsg` uses `#[serde(tag = "type")]` on the
    /// JSON path; we hand-encode it as a plain field so all three formats
    /// see the same on-wire shape.
    #[serde(rename = "type")]
    kind: String,
    id: String,
    #[serde(rename = "fn")]
    fn_name: String,
    args: Vec<ArgValue>,
}

#[derive(SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone)]
enum ArgValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

/// Build a synthetic `CallFrame` with `body_bytes` worth of string args.
/// Mirrors a chatty IPC where 9 small args precede 1 big string (the
/// shape that hits the dispatch hot path on streaming returns).
fn make_call(body_bytes: usize) -> CallFrame {
    let mut args: Vec<ArgValue> = Vec::with_capacity(10);
    for i in 0..9i64 {
        args.push(ArgValue::Int(i));
    }
    // Pad with a single string arg sized to hit the target body length.
    args.push(ArgValue::Str("x".repeat(body_bytes)));
    CallFrame {
        kind: "call".into(),
        id: "01913f2c-7c4e-7000-9000-deadbeefc0de".into(),
        fn_name: "handler".into(),
        args,
    }
}

// ---------------------------------------------------------------------------
// Benches
// ---------------------------------------------------------------------------

fn bench_serialize(c: &mut Criterion) {
    let mut g = c.benchmark_group("ipc_serialize");
    for &size in &[64usize, 4 * 1024, 64 * 1024] {
        let frame = make_call(size);
        g.throughput(Throughput::Bytes(size as u64));

        g.bench_with_input(BenchmarkId::new("json", size), &frame, |b, f| {
            b.iter(|| {
                let v = serde_json::to_vec(black_box(f)).unwrap();
                black_box(v.len());
            });
        });
        g.bench_with_input(BenchmarkId::new("cbor", size), &frame, |b, f| {
            b.iter(|| {
                let mut out: Vec<u8> = Vec::with_capacity(size + 256);
                ciborium::ser::into_writer(black_box(f), &mut out).unwrap();
                black_box(out.len());
            });
        });
        g.bench_with_input(BenchmarkId::new("rkyv", size), &frame, |b, f| {
            b.iter(|| {
                let bytes = rkyv::to_bytes::<RkyvError>(black_box(f)).unwrap();
                black_box(bytes.len());
            });
        });
    }
    g.finish();
}

fn bench_deserialize(c: &mut Criterion) {
    let mut g = c.benchmark_group("ipc_deserialize");
    for &size in &[64usize, 4 * 1024, 64 * 1024] {
        let frame = make_call(size);
        let json = serde_json::to_vec(&frame).unwrap();
        let mut cbor: Vec<u8> = Vec::new();
        ciborium::ser::into_writer(&frame, &mut cbor).unwrap();
        let rkyv_bytes = rkyv::to_bytes::<RkyvError>(&frame).unwrap();
        g.throughput(Throughput::Bytes(size as u64));

        g.bench_with_input(BenchmarkId::new("json", size), &json, |b, bytes| {
            b.iter(|| {
                let f: CallFrame = serde_json::from_slice(black_box(bytes)).unwrap();
                black_box(f.args.len());
            });
        });
        g.bench_with_input(BenchmarkId::new("cbor", size), &cbor, |b, bytes| {
            b.iter(|| {
                let f: CallFrame = ciborium::de::from_reader(black_box(&bytes[..])).unwrap();
                black_box(f.args.len());
            });
        });
        g.bench_with_input(
            BenchmarkId::new("rkyv_zerocopy", size),
            &rkyv_bytes,
            |b, bytes| {
                b.iter(|| {
                    // rkyv zero-copy: cast bytes → archived view (no
                    // deserialization). The fairest "decode" comparison
                    // because in plugin dispatch we never need the owned
                    // Rust struct, only field reads.
                    let archived = rkyv::access::<ArchivedCallFrame, RkyvError>(black_box(bytes))
                        .unwrap();
                    black_box(archived.args.len());
                });
            },
        );
    }
    g.finish();
}

fn bench_wire_size(c: &mut Criterion) {
    // Not a perf bench — just print wire sizes once to make the
    // 30-50% wire-size win in the OPTIMIZATION doc auditable from
    // the bench output. Criterion will report ~0 ns; the value is
    // the `eprintln!` table below.
    let mut g = c.benchmark_group("ipc_wire_size_table");
    for &size in &[64usize, 4 * 1024, 64 * 1024] {
        let frame = make_call(size);
        let json_len = serde_json::to_vec(&frame).unwrap().len();
        let mut cbor: Vec<u8> = Vec::new();
        ciborium::ser::into_writer(&frame, &mut cbor).unwrap();
        let cbor_len = cbor.len();
        let rkyv_len = rkyv::to_bytes::<RkyvError>(&frame).unwrap().len();
        eprintln!(
            "[wire-size] body~{size:>6}B  json={json_len:>7}B  cbor={cbor_len:>7}B \
             ({:>+5.1}%)  rkyv={rkyv_len:>7}B ({:>+5.1}%)",
            (cbor_len as f64 / json_len as f64 - 1.0) * 100.0,
            (rkyv_len as f64 / json_len as f64 - 1.0) * 100.0,
        );
        g.bench_with_input(BenchmarkId::new("noop_marker", size), &frame, |b, _| {
            b.iter(|| black_box(0u32));
        });
    }
    g.finish();
}

criterion_group!(benches, bench_serialize, bench_deserialize, bench_wire_size);
criterion_main!(benches);
