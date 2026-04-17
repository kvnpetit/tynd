import path from "node:path"
import type { TyndConfig } from "../lib/config.ts"
import { getArch, getPlatform } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"
import { loadPackageJson, normalizeAuthor } from "../lib/pkg.ts"
import type { BundleContext } from "./types.ts"

export interface BuildContextOpts {
  cwd: string
  cfg: TyndConfig
  inputBinary: string
  outDir: string
  iconSource: string | null
}

export async function buildBundleContext(opts: BuildContextOpts): Promise<BundleContext> {
  const bundle = opts.cfg.bundle
  if (!bundle) {
    throw new Error(
      'bundle config missing — add a `bundle: { identifier: "com.example.myapp" }` block to tynd.config.ts',
    )
  }

  const pkg = await loadPackageJson(opts.cwd)
  const pkgName = pkg?.name ?? path.basename(opts.cwd)
  const author = normalizeAuthor(pkg?.author)

  const shortDescription =
    bundle.shortDescription ?? pkg?.description ?? bundle.displayName ?? pkgName
  const copyright =
    bundle.copyright ?? (author ? `© ${new Date().getFullYear()} ${author.name}` : "")

  // .ico is a Windows-only format that icns/PNG pipelines can't read. Drop it
  // so bundlers render without an icon rather than crash mid-build.
  let iconSource = opts.iconSource
  if (iconSource && path.extname(iconSource).toLowerCase() === ".ico") {
    log.warn("bundle icons skipped — source is .ico; add a .png or .svg for bundle artwork")
    iconSource = null
  }

  return {
    cwd: opts.cwd,
    inputBinary: opts.inputBinary,
    outDir: opts.outDir,
    appName: slugify(pkgName),
    displayName: bundle.displayName ?? prettyName(pkgName),
    version: pkg?.version ?? "0.0.0",
    identifier: bundle.identifier,
    iconSource,
    categories: bundle.categories ?? [],
    shortDescription,
    longDescription: bundle.longDescription ?? shortDescription,
    copyright,
    author,
    homepage: pkg?.homepage ?? null,
    platform: getPlatform(),
    arch: getArch(),
    toolsDir: path.join(opts.cwd, ".tynd", "cache", "tools"),
    bundleConfig: bundle,
  }
}

function slugify(name: string): string {
  // Strip npm scope, then lowercase, replace non-alnum with hyphen, collapse.
  return name
    .replace(/^@[^/]+\//, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
}

function prettyName(name: string): string {
  const stripped = name.replace(/^@[^/]+\//, "")
  return stripped
    .split(/[-_\s]+/)
    .filter(Boolean)
    .map((w) => w[0]!.toUpperCase() + w.slice(1))
    .join(" ")
}
