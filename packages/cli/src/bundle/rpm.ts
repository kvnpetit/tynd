import { chmodSync, existsSync, mkdirSync, rmSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { rasterSource, renderHicolorSet } from "./icon-gen.ts"
import type { BundleContext } from "./types.ts"

// RPM is the one format without a portable builder — we shell out to the
// system rpmbuild and fail fast if it's missing.
export async function bundleRpm(ctx: BundleContext): Promise<string> {
  if (ctx.platform !== "linux") {
    throw new Error(".rpm bundles can only be built on a Linux host")
  }

  if (!(await hasRpmbuild())) {
    throw new Error(
      "`rpmbuild` not found on PATH.\n" +
        "         -> Debian/Ubuntu: sudo apt install rpm\n" +
        "         -> Fedora/RHEL:   sudo dnf install rpm-build",
    )
  }

  const rpmArch = ctx.arch === "arm64" ? "aarch64" : "x86_64"

  const workDir = path.join(ctx.outDir, `.${ctx.appName}-rpm`)
  const topdir = path.join(workDir, "TOP")
  const sources = path.join(topdir, "SOURCES")

  if (existsSync(workDir)) rmSync(workDir, { recursive: true, force: true })
  for (const sub of ["SPECS", "SOURCES", "BUILD", "BUILDROOT", "RPMS", "SRPMS"]) {
    mkdirSync(path.join(topdir, sub), { recursive: true })
  }

  // Sources that %install copies into %{buildroot}. One file per Source* entry
  // declared in renderSpec: binary, desktop, then one PNG per hicolor size.
  const binDest = path.join(sources, ctx.appName)
  await Bun.write(binDest, Bun.file(ctx.inputBinary))
  chmodSync(binDest, 0o755)
  await Bun.write(path.join(sources, `${ctx.appName}.desktop`), renderDesktopEntry(ctx))

  let iconSizes: number[] = []
  const iconSrc = rasterSource(ctx.iconSource, "rpm")
  if (iconSrc) {
    const hicolor = await renderHicolorSet(iconSrc, ctx.appName)
    iconSizes = hicolor.map((h) => h.size)
    for (const h of hicolor) {
      await Bun.write(path.join(sources, `${ctx.appName}-${h.size}.png`), h.data)
    }
  }

  const specPath = path.join(topdir, "SPECS", `${ctx.appName}.spec`)
  await Bun.write(specPath, renderSpec(ctx, iconSizes))

  await exec("rpmbuild", ["--define", `_topdir ${topdir}`, "--target", rpmArch, "-bb", specPath], {
    silent: true,
  })

  const rpmSrc = path.join(
    topdir,
    "RPMS",
    rpmArch,
    `${ctx.appName}-${ctx.version}-1.${rpmArch}.rpm`,
  )
  if (!existsSync(rpmSrc)) {
    throw new Error(`rpmbuild finished but expected output missing: ${rpmSrc}`)
  }

  const outFile = path.join(ctx.outDir, path.basename(rpmSrc))
  if (existsSync(outFile)) rmSync(outFile, { force: true })
  await Bun.write(outFile, Bun.file(rpmSrc))
  rmSync(workDir, { recursive: true, force: true })

  log.success(`RPM      -> ${log.cyan(`release/${path.basename(outFile)}`)}`)
  return outFile
}

async function hasRpmbuild(): Promise<boolean> {
  try {
    const r = Bun.spawnSync(["which", "rpmbuild"], { stdout: "pipe", stderr: "pipe" })
    return r.exitCode === 0
  } catch {
    return false
  }
}

function renderSpec(ctx: BundleContext, iconSizes: readonly number[]): string {
  const license = ctx.bundleConfig.rpm?.license ?? "Unspecified"
  const requires = ctx.bundleConfig.rpm?.requires ?? []
  const url = ctx.homepage ? `URL: ${ctx.homepage}` : ""
  const requiresLines = requires.map((r) => `Requires: ${r}`).join("\n")

  // One Source line + install + %files line per hicolor size rendered.
  const iconSources = iconSizes.map((s, i) => `Source${i + 2}: ${ctx.appName}-${s}.png`).join("\n")
  const iconInstalls = iconSizes
    .map(
      (s, i) =>
        `install -Dm644 %{SOURCE${i + 2}} %{buildroot}/usr/share/icons/hicolor/${s}x${s}/apps/%{name}.png`,
    )
    .join("\n")
  const iconFiles = iconSizes
    .map((s) => `/usr/share/icons/hicolor/${s}x${s}/apps/%{name}.png`)
    .join("\n")

  return `Name: ${ctx.appName}
Version: ${ctx.version}
Release: 1
Summary: ${ctx.shortDescription}
License: ${license}
${url}
Source0: ${ctx.appName}
Source1: ${ctx.appName}.desktop
${iconSources}
${requiresLines}

%description
${ctx.longDescription}

%install
install -Dm755 %{SOURCE0} %{buildroot}/usr/bin/%{name}
install -Dm644 %{SOURCE1} %{buildroot}/usr/share/applications/%{name}.desktop
${iconInstalls}

%files
/usr/bin/%{name}
/usr/share/applications/%{name}.desktop
${iconFiles}
`
}

function renderDesktopEntry(ctx: BundleContext): string {
  const cats = ctx.categories.length > 0 ? `${ctx.categories.join(";")};` : "Utility;"
  const lines = [
    "[Desktop Entry]",
    "Type=Application",
    `Name=${ctx.displayName}`,
    `Comment=${ctx.shortDescription}`,
    `Exec=/usr/bin/${ctx.appName} %U`,
    `Icon=${ctx.appName}`,
    "Terminal=false",
    `Categories=${cats}`,
  ]
  if (ctx.protocols.length > 0) {
    lines.push(`MimeType=${ctx.protocols.map((s) => `x-scheme-handler/${s}`).join(";")};`)
  }
  lines.push("")
  return lines.join("\n")
}
