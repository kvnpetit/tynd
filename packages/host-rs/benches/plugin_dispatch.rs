//! Microbenchmark for `plugins::dispatch` — the registry route from an
//! IPC `os_call` with `api: "plugin"` to a registered plugin's
//! `dispatch()` method.
//!
//! The registry is process-global + sealed at startup, so we register a
//! mock plugin once in `bench` and then time the dispatch hot path. The
//! plugin itself does no real work (returns a static value); we want to
//! measure the registry lookup + cx wiring + serde_json::Value clone,
//! not whatever a real plugin does on top.
//!
//! Baseline (fill in after first `cargo bench` run):
//!   plugin_dispatch/echo  ~ ?? ns
//!
//! Run with:
//!   cargo bench -p tynd-host --bench plugin_dispatch

use std::sync::Once;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::{json, Value};
use tynd_host::plugin_sdk::{PluginCx, PluginResult, TyndPlugin};
use tynd_host::plugins;

struct BenchPlugin;

impl TyndPlugin for BenchPlugin {
    fn name(&self) -> &'static str {
        "bench-plugin"
    }

    fn dispatch(&self, _cx: &PluginCx, method: &str, _args: Value) -> PluginResult<Value> {
        // Return a small constant payload — we want the registry+cx
        // overhead, not whatever business work a real plugin does.
        Ok(json!({ "echoed": method }))
    }
}

static INIT: Once = Once::new();

fn ensure_registered() {
    INIT.call_once(|| {
        // `install_default_emit` wires the shared emit + dispatch
        // closures so the registered plugin's `init` hook runs.
        plugins::install_default_emit();
        plugins::register(Box::new(BenchPlugin));
    });
}

fn bench_dispatch(c: &mut Criterion) {
    ensure_registered();
    let args = json!({ "k": "v" });
    c.bench_function("plugin_dispatch/echo", |b| {
        b.iter(|| {
            let r = plugins::dispatch(black_box("bench-plugin:ping"), black_box(&args)).unwrap();
            black_box(r);
        });
    });
}

criterion_group!(benches, bench_dispatch);
criterion_main!(benches);
