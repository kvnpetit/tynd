//! Microbenchmark: U1 Layer 2 — rkyv plugin args zero-copy.
//!
//! Compares three paths on the same plugin call shape:
//!
//!   1. **JSON**  — `dispatch(method, Value)` — what shipped before U1.
//!      Pays a `serde_json::from_value` per arg + a `to_value` on the
//!      reply. This is the baseline.
//!   2. **CBOR**  — same `dispatch(method, Value)`, but the args were
//!      transferred as CBOR on the wire. Modeled here as an extra
//!      `ciborium::de::from_reader → Value → from_value` round trip
//!      to mirror what the Bun↔host bridge pays today (Layer 1).
//!   3. **rkyv**  — `dispatch_rkyv(method, &[u8])` reads typed args
//!      directly from the archived buffer (zero-copy) and returns
//!      a typed reply already encoded as rkyv bytes — no serde at all.
//!
//! Three payload sizes per scenario: small ~64 B, medium ~4 KB, large
//! ~64 KB. The string field on `EchoArgs` carries the body to push the
//! decoder into the realistic per-byte regime instead of the
//! pure-fixed-shape best case.
//!
//! Run:
//!     cargo bench -p tynd-host --bench plugin_dispatch_rkyv
//!
//! Quick smoke (skip Criterion warm-up):
//!     cargo bench -p tynd-host --bench plugin_dispatch_rkyv -- \
//!         --sample-size 10 --measurement-time 2

use std::sync::{Arc, Once};

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use rkyv::{rancor::Error as RkyvError, Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_json::{json, Value};
use tynd_host::{
    plugin_sdk::{rkyv_decode_args, rkyv_encode_result, PluginCx, PluginError, PluginResult, TyndPlugin},
    plugins,
};

// ---------------------------------------------------------------------------
// Args / result shape — held in one type so all three paths see the
// same structural cost. Mirrors the smallest realistic plugin call:
// `{ payload: String }` in, `{ length: u64 }` out.
// ---------------------------------------------------------------------------

#[derive(
    SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone,
)]
struct EchoArgs {
    payload: String,
}

#[derive(
    SerdeSerialize, SerdeDeserialize, Archive, RkyvSerialize, RkyvDeserialize, Clone,
)]
struct EchoResult {
    length: u64,
}

// ---------------------------------------------------------------------------
// Mock plugin — implements both `dispatch` (JSON) and `dispatch_rkyv`
// so a single `register(...)` covers all three benches.
// ---------------------------------------------------------------------------

struct EchoPlugin;

impl TyndPlugin for EchoPlugin {
    fn name(&self) -> &'static str {
        "bench-rkyv"
    }

    fn dispatch(&self, _cx: &PluginCx, method: &str, args: Value) -> PluginResult<Value> {
        if method != "echo" {
            return Err(PluginError::Custom(format!("unknown method: {method}")));
        }
        // Pay the JSON path's typed-decode cost — the plugin walks
        // `args.get("payload")` instead of materializing `EchoArgs`,
        // because that's exactly what every existing plugin does today.
        let payload = args
            .get("payload")
            .and_then(Value::as_str)
            .ok_or_else(|| PluginError::MissingArg {
                key: "payload".into(),
            })?;
        Ok(json!({ "length": payload.len() as u64 }))
    }

    fn dispatch_rkyv(
        &self,
        _cx: &PluginCx,
        method: &str,
        args: &[u8],
    ) -> Option<PluginResult<Vec<u8>>> {
        if method != "echo" {
            return None;
        }
        // Zero-copy field read: cast bytes → archived view, read
        // `payload.len()` directly off the validated buffer, then
        // re-encode the typed reply. No serde, no `Value`.
        let archived = match rkyv::access::<ArchivedEchoArgs, RkyvError>(args) {
            Ok(v) => v,
            Err(e) => return Some(Err(PluginError::Custom(format!("rkyv decode: {e}")))),
        };
        let length = archived.payload.as_ref().len() as u64;
        Some(rkyv_encode_result(&EchoResult { length }))
    }
}

static INIT: Once = Once::new();

fn ensure_registered() {
    INIT.call_once(|| {
        plugins::install_default_emit();
        plugins::register(Arc::new(EchoPlugin));
    });
}

// ---------------------------------------------------------------------------
// Payload builders — produce the three size classes the bench iterates
// over. Wrapping happens inside each variant so the per-bench cost is
// only the encode / call / decode steps, not the data construction.
// ---------------------------------------------------------------------------

fn make_args(body_bytes: usize) -> EchoArgs {
    EchoArgs {
        payload: "x".repeat(body_bytes),
    }
}

const SIZES: &[usize] = &[64, 4 * 1024, 64 * 1024];

// ---------------------------------------------------------------------------
// Bench 1 — JSON dispatch, the legacy path.
// ---------------------------------------------------------------------------

fn bench_json(c: &mut Criterion) {
    ensure_registered();
    let mut g = c.benchmark_group("plugin_dispatch_rkyv/json");
    for &size in SIZES {
        let args = make_args(size);
        // Pre-build the JSON template so the per-iter cost is dispatch,
        // not the `to_value` + `clone` setup. We still pay one clone
        // per iteration because `dispatch` takes args by value.
        let template = serde_json::to_value(&args).unwrap();
        g.throughput(Throughput::Bytes(size as u64));

        g.bench_with_input(BenchmarkId::from_parameter(size), &template, |b, t| {
            b.iter(|| {
                let r = plugins::dispatch(
                    black_box("bench-rkyv:echo"),
                    black_box(t.clone()),
                )
                .unwrap();
                black_box(r);
            });
        });
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Bench 2 — CBOR-on-wire, JSON-at-plugin: the U1 Layer 1 picture for
// plugin args. The plugin is unchanged (still consumes a `Value`); the
// extra cost is the CBOR → Value step the dispatcher would have to pay.
// ---------------------------------------------------------------------------

fn bench_cbor_to_json(c: &mut Criterion) {
    ensure_registered();
    let mut g = c.benchmark_group("plugin_dispatch_rkyv/cbor_to_json");
    for &size in SIZES {
        let args = make_args(size);
        let mut cbor: Vec<u8> = Vec::new();
        ciborium::ser::into_writer(&args, &mut cbor).unwrap();
        g.throughput(Throughput::Bytes(size as u64));

        g.bench_with_input(BenchmarkId::from_parameter(size), &cbor, |b, bytes| {
            b.iter(|| {
                // Decode CBOR bytes → Value (the JSON-bridge cost).
                let v: Value = ciborium::de::from_reader(black_box(&bytes[..])).unwrap();
                let r = plugins::dispatch(black_box("bench-rkyv:echo"), v).unwrap();
                black_box(r);
            });
        });
    }
    g.finish();
}

// ---------------------------------------------------------------------------
// Bench 3 — rkyv zero-copy: the Layer 2 path. Args arrive as already-
// archived bytes; the plugin's `dispatch_rkyv` reads the field
// directly off the buffer and returns rkyv-encoded reply bytes.
// ---------------------------------------------------------------------------

fn bench_rkyv_zero_copy(c: &mut Criterion) {
    ensure_registered();
    let mut g = c.benchmark_group("plugin_dispatch_rkyv/rkyv");
    for &size in SIZES {
        let args = make_args(size);
        let bytes = rkyv_encode_result(&args).unwrap();
        g.throughput(Throughput::Bytes(size as u64));

        // We bypass `plugins::dispatch` for the rkyv path because that
        // function only knows the JSON signature today. The bench
        // calls the plugin's `dispatch_rkyv` directly through the
        // registry — same single atomic load, no JSON detour. The
        // host wiring lands in U12's caller-side step.
        g.bench_with_input(BenchmarkId::from_parameter(size), &bytes, |b, buf| {
            b.iter(|| {
                let plugin: Arc<dyn TyndPlugin> = Arc::new(EchoPlugin);
                let cx = PluginCx::test();
                let r = plugin
                    .dispatch_rkyv(&cx, black_box("echo"), black_box(buf))
                    .expect("rkyv path engaged")
                    .unwrap();
                // Decode the reply so we measure the full round-trip
                // cost (caller side would do the same).
                let decoded: EchoResult = rkyv_decode_args(&r).unwrap();
                black_box(decoded.length);
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench_json, bench_cbor_to_json, bench_rkyv_zero_copy);
criterion_main!(benches);
