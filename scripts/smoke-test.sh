#!/usr/bin/env bash
# Wave 4.4 smoke runner — Linux + macOS.
#
# Launches `tynd-{full,lite} --smoke-mode` against the minimal app under
# scripts/smoke-test-app and asserts:
#   - the binary exists and is executable
#   - it exits cleanly within 10 s (window opens, then closes itself)
#   - exit code is 0
#
# NOTE: --smoke-mode is NOT yet wired in the host Rust binaries (see W4.4
# log entry in IMPLEMENTATION_PROGRESS.md). Until that lands, the workflow
# only runs this script on `workflow_dispatch`. Flip the gate in smoke.yml
# once `tynd-{full,lite} --smoke-mode` self-exits after PageReady.

set -euo pipefail

RUNTIME="${1:-full}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$SCRIPT_DIR/smoke-test-app"
BIN="./target/release/tynd-$RUNTIME"

echo "[smoke] runtime=$RUNTIME bin=$BIN app=$APP_DIR"

if [ ! -x "$BIN" ]; then
    echo "[smoke] ERROR: $BIN not found or not executable" >&2
    exit 1
fi

cd "$APP_DIR"

# `timeout` exits 124 on time-out — distinguish that from genuine binary
# failure so CI logs surface "window stuck open" vs "binary crashed".
EXIT_CODE=0
timeout 10 "$BIN" --smoke-mode 2>&1 || EXIT_CODE=$?

if [ "$EXIT_CODE" -eq 0 ]; then
    echo "[smoke] PASSED for $RUNTIME"
    exit 0
elif [ "$EXIT_CODE" -eq 124 ]; then
    echo "[smoke] ERROR: TIMED OUT — window did not close within 10s" >&2
    exit 1
else
    echo "[smoke] ERROR: FAILED with exit code $EXIT_CODE" >&2
    exit "$EXIT_CODE"
fi
