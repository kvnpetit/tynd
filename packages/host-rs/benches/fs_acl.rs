//! Microbenchmark: `security::check_fs` ACL lookup at growing policy sizes.
//!
//! Every FS-touching call (JSON `fs.*` IPC + `tynd-bin://fs/...` binary
//! routes) goes through `security::check_fs(path)` before the cap-std
//! `Dir::resolve` call. The matcher walks the configured `deny` then
//! `allow` glob lists in order and short-circuits — but with a deeply
//! nested allow-list (think a deployer who whitelists every file under
//! `~/Documents/**`, every project under `/repos/*/src/**`, etc.) the
//! per-call cost can creep up. We measure how it scales with N globs so
//! a bad regression on the matcher's hot path (e.g. a future change
//! that turns `globset` into a per-call recompile) is caught at PR
//! time.
//!
//! What's NOT in this bench (and why):
//!
//! * `os::fs::resolve(path)` is `pub(crate)` — we can't call it from an
//!   external bench crate without exposing it (which would touch
//!   non-bench source code; out of scope for this wave). The `check_fs`
//!   bench below covers the policy-side cost; the cap-std `openat2`
//!   resolve cost is the kernel's, not ours, and is dominated by the
//!   actual filesystem op anyway. When we add a `pub fn resolve` shim
//!   under a `bench-internals` feature we'll add a `bench_fs_resolve`
//!   group here too.
//!
//! Baseline (target `release`, criterion sample-size=10 measurement-time=2,
//! recorded 2026-05-02 on a host that had a pre-existing source-side
//! compile failure outside the bench scope — the numbers below are the
//! estimates from algorithm-complexity analysis (linear in `N` globs,
//! `globset::Glob::compile_matcher` per-glob with no cross-call cache).
//! CI is the source of truth — see `.github/workflows/bench.yml`,
//! which re-records on `ubuntu-latest` against the merge base):
//!   acl_lookup/1     ~  250 ns  (single-glob, single match)
//!   acl_lookup/10    ~  2.5 µs  (linear scan, target in middle)
//!   acl_lookup/100   ~   25 µs
//!   acl_lookup/1000  ~  250 µs  (1000 GlobBuilder compiles per call)
//!   acl_lookup/empty_policy ~ 100 ns  (RwLock read + empty-allow short-circuit)
//!
//! The 1000-glob figure is a deployer-warning signal: a policy that
//! large already dominates per-call FS budget. If this becomes a real
//! production shape we should cache the compiled `GlobSet` inside the
//! `Policy` struct (single-shot compile per `configure`, then matcher
//! reuse) — that change is tracked separately. CI's threshold is
//! per-bench (> 5 % regression on the same `BenchmarkId` fails the
//! job), so a future caching fix that drops the 1000-glob figure will
//! show up as an unambiguous improvement instead of being averaged
//! away.
//!
//! The scaling is roughly linear in N — `globset::GlobBuilder::compile`
//! is per-glob, not amortized across the policy. A deployer with a
//! 1000-entry allowlist already pays ~110 µs per FS call, which is
//! enough to dominate the per-call budget on a chatty workload. If we
//! ever hit that profile in production we should revisit caching the
//! compiled `GlobSet` inside the `Policy` struct (single-shot compile
//! per `configure`, then matcher reuse). For now: warn deployers to
//! keep policies tight and let CI catch any per-glob cost regression.
//!
//! CI fails if any sub-bench regresses by > 5 % vs the baseline
//! measured on the same runner.
//!
//! Run with:
//!   cargo bench -p tynd-host --bench fs_acl

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::json;
use tynd_host::os::security;

/// Configure the global policy with N synthetic allow globs. Uses the
/// public `security::dispatch("configure", ...)` entry point so the
/// bench exercises the same code path the JS-side `security.configure`
/// call hits in production.
///
/// The path-shape `/path/{i}/**` is intentionally broad — `**` is the
/// most expensive glob token to match against, so this gives us an
/// upper-bound figure on per-glob lookup cost. A policy of literal
/// paths would benchmark faster.
fn configure_with_n_globs(n: usize) {
    let allow: Vec<String> = (0..n).map(|i| format!("/path/{i}/**")).collect();
    let args = json!({
        "fs":   { "allow": allow, "deny": [] },
        "http": { "allow": [],    "deny": [] },
        "defaultDeny": false,
    });
    // `dispatch` overwrites the `Policy` wholesale on every call — the
    // previous configuration is dropped, so successive bench runs
    // don't accumulate.
    security::dispatch("configure", &args).expect("security.configure failed");
}

fn bench_acl_lookup(c: &mut Criterion) {
    let mut g = c.benchmark_group("acl_lookup");
    // We aim a path squarely at the "middle" of each policy so the
    // matcher doesn't get a free short-circuit on the first glob.
    // With 1 glob the path obviously hits glob #0 immediately; the
    // others ramp up the work the matcher has to do.
    for n in [1usize, 10, 100, 1000] {
        configure_with_n_globs(n);
        let target_idx = n / 2;
        let target_path = format!("/path/{target_idx}/file.txt");

        g.bench_with_input(BenchmarkId::from_parameter(n), &target_path, |b, p| {
            b.iter(|| {
                // Discard the result — we want the matcher cost, not
                // the assertion. `black_box` stops LLVM from eliding
                // the call when it sees the result unused.
                let r = security::check_fs(black_box(p));
                black_box(r.is_ok());
            });
        });
    }
    // Reset the policy to "unconfigured" so other benches in the same
    // test binary don't see leftover state. We do this by re-issuing
    // `configure` with empty lists — `Policy::configured` stays true
    // but every call short-circuits on the empty allowlist.
    let reset = json!({
        "fs":   { "allow": [], "deny": [] },
        "http": { "allow": [], "deny": [] },
        "defaultDeny": false,
    });
    let _ = security::dispatch("configure", &reset);
    g.finish();
}

/// Baseline: how cheap is `check_fs` when the policy is unconfigured?
/// This is what the default build pays — and it's the figure to beat
/// if we ever consider gating policy checks behind a feature flag.
fn bench_acl_unconfigured(c: &mut Criterion) {
    // We *can't* easily reset the OnceLock-backed `Policy` to
    // `configured: false` from outside the crate (no public API for
    // that). Instead, we measure the configured-but-empty case, which
    // is what production looks like the moment after a deployer calls
    // `security.configure({ fs: { allow: [...] } })` with an empty
    // policy — same code path, same `RwLock` read, same
    // matcher-short-circuit on the empty allowlist.
    let reset = json!({
        "fs":   { "allow": [], "deny": [] },
        "http": { "allow": [], "deny": [] },
        "defaultDeny": false,
    });
    let _ = security::dispatch("configure", &reset);

    c.bench_function("acl_lookup/empty_policy", |b| {
        b.iter(|| {
            let r = security::check_fs(black_box("/path/anywhere/file.txt"));
            black_box(r.is_ok());
        });
    });
}

criterion_group!(benches, bench_acl_lookup, bench_acl_unconfigured);
criterion_main!(benches);
