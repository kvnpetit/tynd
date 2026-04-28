// Backend entry for the smoke fixture. `app.start()` is the only thing
// we need: when `TYND_SMOKE=1` is set, core's start() bypasses the IPC
// loop and exits with code 0 right after firing the lifecycle hooks.
// This lets CI verify the build + launch round-trip without a display.
import { app } from "@tynd/core";

let readyFired = false;
let closeFired = false;

app.onReady(() => {
  readyFired = true;
  // Surface to stderr so the test can scrape if it ever wants to —
  // stdout is reserved for the IPC frame in normal runs.
  console.error("[smoke] onReady fired");
});

app.onClose(() => {
  closeFired = true;
  console.error("[smoke] onClose fired");
});

app.start({
  window: { title: "Tynd Smoke", width: 320, height: 200 },
});

// Defensive: if for any reason TYND_SMOKE didn't kick in, log it. The
// host expects the IPC loop here, but for the smoke path we should
// never reach this line.
if (!readyFired || !closeFired) {
  console.error("[smoke] WARN: lifecycle hooks did not fire synchronously");
}
