#!/usr/bin/env bash
# Compile-check the tynd-host feature matrix so gated APIs (api-menu, api-tray,
# api-multiwindow, ...) never rot in the build configurations nobody ships by
# default. Usage: scripts/check-host-feature-matrix.sh [clippy]
#   (no arg)  cargo check per row
#   clippy    cargo clippy -D warnings per row
set -euo pipefail

cmd=(cargo check --locked)
tail_args=()
if [[ "${1:-}" == "clippy" ]]; then
  cmd=(cargo clippy --locked)
  tail_args=(-- -D warnings)
fi

# The non-UI system-plugin APIs that used to ship together.
plugins_minus_ui="api-clipboard,api-compute,api-dialog,api-shell,api-single-instance,api-store,api-theme,api-keys,api-cli"

rows=(
  ""                                                  # core only (preset host)
  "api-menu"
  "api-tray"
  "api-multiwindow"
  "${plugins_minus_ui}"                               # no menu/tray/multiwindow
  "${plugins_minus_ui},api-multiwindow"               # no menu + tray
  "${plugins_minus_ui},api-menu,api-multiwindow"      # no tray
  "${plugins_minus_ui},api-tray,api-multiwindow"      # no menu
  "${plugins_minus_ui},api-menu,api-tray,api-multiwindow"
  "debug-protocol"                                    # debug without xcap on macOS/Linux
  "debug-screenshot"
)

for features in "${rows[@]}"; do
  echo "==> tynd-host --no-default-features --features '${features}'"
  "${cmd[@]}" -p tynd-host --no-default-features --features "${features}" ${tail_args[@]+"${tail_args[@]}"}
done

echo "==> tynd-host (default features)"
"${cmd[@]}" -p tynd-host ${tail_args[@]+"${tail_args[@]}"}
