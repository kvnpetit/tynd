import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import { exec } from "../lib/exec.ts"
import { log } from "../lib/logger.ts"
import { loadIconAsPng } from "./icon-gen.ts"
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

  // Sources that %install copies into %{buildroot}. Named to match the
  // Source0/Source1/Source2 entries emitted by renderSpec.
  writeFileSync(path.join(sources, ctx.appName), readFileSync(ctx.inputBinary), { mode: 0o755 })
  writeFileSync(path.join(sources, `${ctx.appName}.desktop`), renderDesktopEntry(ctx), {
    mode: 0o644,
  })
  if (ctx.iconSource) {
    writeFileSync(path.join(sources, `${ctx.appName}.png`), await loadIconAsPng(ctx.iconSource), {
      mode: 0o644,
    })
  }

  const specPath = path.join(topdir, "SPECS", `${ctx.appName}.spec`)
  writeFileSync(specPath, renderSpec(ctx, !!ctx.iconSource))

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
  writeFileSync(outFile, readFileSync(rpmSrc))
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

function renderSpec(ctx: BundleContext, hasIcon: boolean): string {
  const license = ctx.bundleConfig.rpm?.license ?? "Unspecified"
  const requires = ctx.bundleConfig.rpm?.requires ?? []
  const url = ctx.homepage ? `URL: ${ctx.homepage}` : ""
  const requiresLines = requires.map((r) => `Requires: ${r}`).join("\n")
  const iconInstall = hasIcon
    ? `install -Dm644 %{SOURCE2} %{buildroot}/usr/share/icons/hicolor/256x256/apps/%{name}.png`
    : ""
  const iconFile = hasIcon ? `/usr/share/icons/hicolor/256x256/apps/%{name}.png` : ""
  const source2 = hasIcon ? `Source2: ${ctx.appName}.png` : ""

  return `Name: ${ctx.appName}
Version: ${ctx.version}
Release: 1
Summary: ${ctx.shortDescription}
License: ${license}
${url}
Source0: ${ctx.appName}
Source1: ${ctx.appName}.desktop
${source2}
${requiresLines}

%description
${ctx.longDescription}

%install
install -Dm755 %{SOURCE0} %{buildroot}/usr/bin/%{name}
install -Dm644 %{SOURCE1} %{buildroot}/usr/share/applications/%{name}.desktop
${iconInstall}

%files
/usr/bin/%{name}
/usr/share/applications/%{name}.desktop
${iconFile}
`
}

function renderDesktopEntry(ctx: BundleContext): string {
  const cats = ctx.categories.length > 0 ? `${ctx.categories.join(";")};` : "Utility;"
  return [
    "[Desktop Entry]",
    "Type=Application",
    `Name=${ctx.displayName}`,
    `Comment=${ctx.shortDescription}`,
    `Exec=/usr/bin/${ctx.appName}`,
    `Icon=${ctx.appName}`,
    "Terminal=false",
    `Categories=${cats}`,
    "",
  ].join("\n")
}
