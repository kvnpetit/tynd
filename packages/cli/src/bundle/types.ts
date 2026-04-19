import type { BundleConfig } from "../lib/config.ts"
import type { Arch, Platform } from "../lib/detect.ts"

export type BundleTarget = "app" | "dmg" | "deb" | "rpm" | "appimage" | "nsis" | "msi"

export const ALL_TARGETS: readonly BundleTarget[] = [
  "app",
  "dmg",
  "deb",
  "rpm",
  "appimage",
  "nsis",
  "msi",
]

export const TARGETS_BY_PLATFORM: Record<Platform, readonly BundleTarget[]> = {
  macos: ["app", "dmg"],
  linux: ["deb", "rpm", "appimage"],
  windows: ["nsis", "msi"],
}

export interface BundleContext {
  cwd: string
  inputBinary: string
  outDir: string
  /** Slug — safe for filenames, .desktop Name, PkgInfo. */
  appName: string
  /** Human-facing name shown in Finder, Start Menu, etc. */
  displayName: string
  version: string
  /** Reverse-DNS (e.g. com.example.myapp). */
  identifier: string
  /** Raw icon source path (.svg, .png or .ico). Each bundler handles formats it can't use. */
  iconSource: string | null
  categories: readonly string[]
  shortDescription: string
  longDescription: string
  copyright: string
  author: { name: string; email?: string; url?: string } | null
  homepage: string | null
  platform: Platform
  arch: Arch
  toolsDir: string
  bundleConfig: BundleConfig
}
