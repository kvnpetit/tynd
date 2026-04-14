#!/usr/bin/env bun
/**
 * Build the native webview-host binary via `cargo build --release`.
 *
 * Prerequisites:
 *   All platforms  — Rust toolchain (https://rustup.rs)
 *   Windows        — no extra deps (WebView2 runtime ships with Windows 11 / Edge)
 *   macOS          — xcode-select --install
 *   Linux          — sudo apt install libgtk-3-dev libsoup2.4-dev libjavascriptcoregtk-4.1-dev libwebkit2gtk-4.1-dev
 *
 * Cross-compilation targets (set via BUNVIEW_TARGET env var):
 *   windows-x64    windows-arm64
 *   linux-x64      linux-arm64
 *   macos-x64      macos-arm64
 *
 * For cross-compilation you need the corresponding Rust target installed:
 *   rustup target add aarch64-unknown-linux-gnu
 *   rustup target add aarch64-apple-darwin
 *   etc.
 */

import path from "path";
import { $ } from "bun";

const ROOT      = path.join(import.meta.dir, "..");
const HOST_RS   = path.join(ROOT, "host-rs");

// ── Target resolution ─────────────────────────────────────────────────────────

type Target =
  | "windows-x64" | "windows-arm64"
  | "linux-x64"   | "linux-arm64"
  | "macos-x64"   | "macos-arm64";

function autoTarget(): Target {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  if (process.platform === "win32")  return `windows-${arch}`;
  if (process.platform === "darwin") return `macos-${arch}`;
  return `linux-${arch}`;
}

const TARGET: Target = (process.env.BUNVIEW_TARGET as Target) ?? autoTarget();
const [TARGET_OS, TARGET_ARCH] = TARGET.split("-") as [string, string];

const EXT     = TARGET_OS === "windows" ? ".exe" : "";
const BIN_DIR = path.join(ROOT, "bin", TARGET);
const OUTPUT  = path.join(BIN_DIR, `webview-host${EXT}`);

// ── Rust triple mapping ───────────────────────────────────────────────────────

function rustTriple(target: Target): string | null {
  const map: Record<Target, string> = {
    "windows-x64":  "x86_64-pc-windows-msvc",
    "windows-arm64":"aarch64-pc-windows-msvc",
    "linux-x64":    "x86_64-unknown-linux-gnu",
    "linux-arm64":  "aarch64-unknown-linux-gnu",
    "macos-x64":    "x86_64-apple-darwin",
    "macos-arm64":  "aarch64-apple-darwin",
  };
  return map[target] ?? null;
}

// ── Main ──────────────────────────────────────────────────────────────────────

async function main() {
  console.log(`[bunview] Building webview-host (Rust) → ${TARGET}`);

  // Check Rust is installed
  try {
    await $`cargo --version`.quiet();
  } catch {
    console.error(
      "[bunview] ❌ cargo not found. Install Rust: https://rustup.rs\n" +
      "         curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh",
    );
    process.exit(1);
  }

  // Determine if we're cross-compiling
  const hostOS   = process.platform === "win32" ? "windows" : process.platform === "darwin" ? "macos" : "linux";
  const hostArch = process.arch === "arm64" ? "arm64" : "x64";
  const isCross  = TARGET_OS !== hostOS || TARGET_ARCH !== hostArch;

  const triple = rustTriple(TARGET);
  if (!triple) {
    console.error(`[bunview] ❌ Unknown target: ${TARGET}`);
    process.exit(1);
  }

  // Install target if cross-compiling
  if (isCross) {
    console.log(`[bunview] Installing Rust target ${triple}…`);
    await $`rustup target add ${triple}`;
  }

  // Build
  const releaseDir = path.join(HOST_RS, "target", triple, "release");
  const srcBin     = path.join(releaseDir, `webview-host${EXT}`);

  if (isCross) {
    await $`cargo build --release --target ${triple}`.cwd(HOST_RS);
  } else {
    // Native build — binary lands in target/release/
    const nativeRelease = path.join(HOST_RS, "target", "release");
    await $`cargo build --release`.cwd(HOST_RS);
    const nativeBin = path.join(nativeRelease, `webview-host${EXT}`);

    await $`mkdir -p ${BIN_DIR}`.quiet();
    await copyBinary(nativeBin, OUTPUT);
    if (process.platform !== "win32") {
      await $`chmod +x ${OUTPUT}`.quiet();
    }
    console.log(`[bunview] ✅ Built: ${OUTPUT}`);
    return;
  }

  // Copy cross-compiled binary
  await $`mkdir -p ${BIN_DIR}`.quiet();
  await copyBinary(srcBin, OUTPUT);
  if (process.platform !== "win32") {
    await $`chmod +x ${OUTPUT}`.quiet();
  }
  console.log(`[bunview] ✅ Built: ${OUTPUT}`);
}

/** Copy a binary file. On Windows, use PowerShell Copy-Item for robustness. */
async function copyBinary(src: string, dest: string): Promise<void> {
  if (process.platform === "win32") {
    // PowerShell Copy-Item works even when the destination is in use by Windows Defender
    // or has a stale handle. Use -Force to overwrite.
    await $`powershell -NoProfile -NonInteractive -Command "Copy-Item -Path '${src}' -Destination '${dest}' -Force"`.quiet();
  } else {
    await Bun.write(dest, Bun.file(src));
  }
}

main().catch((err) => {
  console.error("[bunview] Build failed:", err.message ?? err);
  process.exit(1);
});
