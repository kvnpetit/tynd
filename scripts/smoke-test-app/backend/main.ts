// Wave 4.4 smoke-test backend.
//
// Boots a 400×300 window titled "Smoke Test". When `--smoke-mode` (CLI
// flag) or `TYND_SMOKE=1` (env) is set, schedules a self-exit ~1 s after
// the window reports ready so the host process terminates cleanly with
// exit code 0 — that's what scripts/smoke-test.{sh,ps1} asserts.

import { app } from "@tynd/core";

const isSmoke =
  (typeof process !== "undefined" && (process.argv?.includes("--smoke-mode") ?? false)) ||
  (typeof process !== "undefined" && process.env?.TYND_SMOKE === "1");

app.start({
  window: {
    title: "Smoke Test",
    width: 400,
    height: 300,
    center: true,
  },
});

if (isSmoke) {
  app.onReady(() => {
    // Use process.exit so we don't depend on a backend-side `app.close()`
    // API (none exists today — close lives on the frontend client). The
    // host process traps stdin EOF on full-runtime and exits cleanly when
    // the Bun child exits, which is what we want here.
    setTimeout(() => {
      // biome-ignore lint/suspicious/noConsole: smoke-test diagnostic line — surfaces in CI logs
      console.log("[smoke] window opened, exiting");
      if (typeof process !== "undefined") {
        process.exit(0);
      }
    }, 1000);
  });
}
