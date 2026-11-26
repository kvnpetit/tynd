export const DESKTOP_TARGETS = [
  {
    target: "windows-x64",
    releaseBundlerTarget: "x86_64-pc-windows-msvc",
    releaseBundlerBinary: "tynd-bundler.exe",
    releaseBundlerAsset: "tynd-bundler-x86_64-pc-windows-msvc.zip",
  },
  {
    target: "windows-arm64",
    releaseBundlerTarget: "aarch64-pc-windows-msvc",
    releaseBundlerBinary: "tynd-bundler.exe",
    releaseBundlerAsset: "tynd-bundler-aarch64-pc-windows-msvc.zip",
  },
  {
    target: "linux-x64",
    releaseBundlerTarget: "x86_64-unknown-linux-gnu",
    releaseBundlerBinary: "tynd-bundler",
    releaseBundlerAsset: "tynd-bundler-x86_64-unknown-linux-gnu.tar.gz",
  },
  {
    target: "linux-arm64",
    releaseBundlerTarget: "aarch64-unknown-linux-gnu",
    releaseBundlerBinary: "tynd-bundler",
    releaseBundlerAsset: "tynd-bundler-aarch64-unknown-linux-gnu.tar.gz",
  },
  {
    target: "macos-x64",
    releaseBundlerTarget: "x86_64-apple-darwin",
    releaseBundlerBinary: "tynd-bundler",
    releaseBundlerAsset: "tynd-bundler-x86_64-apple-darwin.tar.gz",
  },
  {
    target: "macos-arm64",
    releaseBundlerTarget: "aarch64-apple-darwin",
    releaseBundlerBinary: "tynd-bundler",
    releaseBundlerAsset: "tynd-bundler-aarch64-apple-darwin.tar.gz",
  },
] as const;
