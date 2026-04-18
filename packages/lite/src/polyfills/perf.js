// performance.now only. mark / measure / getEntriesByName trimmed — dev
// profiling is tooling, not a backend runtime concern. Apps that need it
// can install `perf_hooks` via bun in full mode or track times manually
// with `performance.now()`.
(function () {
  // rquickjs already exposes a frozen `performance.now`; replace the whole
  // object so the bridge version wins.
  globalThis.performance = { now: function () { return __tynd_perf_now__(); } };
})();
