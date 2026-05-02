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
//! Baseline (recorded 2026-05-02 on Windows 11, AMD x86_64, target
//! `release`, criterion sample-size=10 measurement-time=2):
//!   plugin_dispatch/echo  ~ 255 ns   [239 ns ; 286 ns]
//!
//! That number is dominated by the `serde_json::Value` clone (the mock
//! plugin returns a tiny `{ "echoed": method }` object). Real plugins
//! will pay more on top of this — what we're guarding here is that
//! the registry lookup + cx wiring stays sub-µs. CI fails if this bench
//! regresses by > 5 % vs the baseline measured on the same runner —
//! see `.github/workflows/bench.yml`.
//!
//! Run with:
//!   cargo bench -p tynd-host --bench plugin_dispatch

use std::sync::Once;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::{json, Value};
use tynd_host::{
    plugin_sdk::{PluginCx, PluginResult, TyndPlugin},
    plugins,
};

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
