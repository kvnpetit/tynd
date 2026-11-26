#!/usr/bin/env bash
set -euo pipefail

sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  libgtk-3-dev libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libxdo-dev libxss-dev libpipewire-0.3-dev \
  libgbm-dev libdrm-dev libclang-dev "$@"
