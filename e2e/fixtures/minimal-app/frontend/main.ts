// Frontend entry — minimal. The smoke test never opens a real webview
// (TYND_SMOKE bypasses it), so this script's job is just to give the
// build pipeline something non-empty to copy into the bundle.
const status = document.getElementById("status");
if (status) {
  status.textContent = "ready";
}
