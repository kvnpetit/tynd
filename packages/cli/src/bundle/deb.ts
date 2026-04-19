import { existsSync, rmSync } from "node:fs"
import path from "node:path"
import { promisify } from "node:util"
import { gzip as gzipCb } from "node:zlib"
import { log } from "../lib/logger.ts"
import { rasterSource, renderHicolorSet } from "./icon-gen.ts"
import type { BundleContext } from "./types.ts"

const gzip = promisify(gzipCb)

// .deb is an `ar` archive with three members in fixed order:
//   debian-binary + control.tar.gz + data.tar.gz.
export async function bundleDeb(ctx: BundleContext): Promise<string> {
  if (ctx.platform !== "linux") {
    throw new Error(".deb bundles can only be built on a Linux host")
  }

  const debArch = ctx.arch === "arm64" ? "arm64" : "amd64"
  const outFile = path.join(ctx.outDir, `${ctx.appName}_${ctx.version}_${debArch}.deb`)
  if (existsSync(outFile)) rmSync(outFile, { force: true })

  const tarPack = (await import("tar-stream")).default.pack()
  const dataChunks: Buffer[] = []
  tarPack.on("data", (c: Buffer) => dataChunks.push(c))
  const dataDone = new Promise<void>((resolve, reject) => {
    tarPack.on("end", resolve)
    tarPack.on("error", reject)
  })

  const binBytes = Buffer.from(await Bun.file(ctx.inputBinary).bytes())
  let installedSize = binBytes.length

  tarPack.entry({ name: `./usr/bin/${ctx.appName}`, mode: 0o755, type: "file" }, binBytes)

  const desktop = renderDesktopEntry(ctx)
  tarPack.entry(
    { name: `./usr/share/applications/${ctx.appName}.desktop`, mode: 0o644, type: "file" },
    desktop,
  )
  installedSize += desktop.length

  const iconSrc = rasterSource(ctx.iconSource, "deb")
  if (iconSrc) {
    const hicolor = await renderHicolorSet(iconSrc, ctx.appName)
    for (const entry of hicolor) {
      tarPack.entry({ name: `./${entry.relPath}`, mode: 0o644, type: "file" }, entry.data)
      installedSize += entry.data.length
    }
  }

  tarPack.finalize()
  await dataDone
  const dataTarGz = Buffer.from(await gzip(Buffer.concat(dataChunks)))

  const controlPack = (await import("tar-stream")).default.pack()
  const controlChunks: Buffer[] = []
  controlPack.on("data", (c: Buffer) => controlChunks.push(c))
  const controlDone = new Promise<void>((resolve, reject) => {
    controlPack.on("end", resolve)
    controlPack.on("error", reject)
  })
  const controlFile = renderControl(ctx, debArch, Math.ceil(installedSize / 1024))
  controlPack.entry({ name: "./control", mode: 0o644, type: "file" }, controlFile)
  controlPack.finalize()
  await controlDone
  const controlTarGz = Buffer.from(await gzip(Buffer.concat(controlChunks)))

  const deb = Buffer.concat([
    Buffer.from("!<arch>\n", "ascii"),
    arEntry("debian-binary", Buffer.from("2.0\n", "ascii")),
    arEntry("control.tar.gz", controlTarGz),
    arEntry("data.tar.gz", dataTarGz),
  ])

  await Bun.write(outFile, deb)
  log.success(`Deb      -> ${log.cyan(`release/${path.basename(outFile)}`)}`)
  return outFile
}

function renderControl(ctx: BundleContext, arch: string, installedKb: number): Buffer {
  const depends = (ctx.bundleConfig.deb?.depends ?? []).join(", ")
  const section = ctx.bundleConfig.deb?.section ?? "utils"
  const priority = ctx.bundleConfig.deb?.priority ?? "optional"
  const maintainer = ctx.author
    ? `${ctx.author.name}${ctx.author.email ? ` <${ctx.author.email}>` : ""}`
    : "Unknown <unknown@example.com>"

  // Debian policy: description continuation lines must start with a space.
  const longLines = ctx.longDescription
    .split(/\r?\n/)
    .map((l) => (l.trim() === "" ? " ." : ` ${l}`))
    .join("\n")

  const lines = [
    `Package: ${ctx.appName}`,
    `Version: ${ctx.version}`,
    `Architecture: ${arch}`,
    `Maintainer: ${maintainer}`,
    `Installed-Size: ${installedKb}`,
    depends ? `Depends: ${depends}` : null,
    `Section: ${section}`,
    `Priority: ${priority}`,
    ctx.homepage ? `Homepage: ${ctx.homepage}` : null,
    `Description: ${ctx.shortDescription}`,
    longLines,
  ].filter(Boolean)

  return Buffer.from(`${lines.join("\n")}\n`, "utf8")
}

function renderDesktopEntry(ctx: BundleContext): Buffer {
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
  return Buffer.from(`${lines.join("\n")}\n`, "utf8")
}

// SysV ar member: 60-byte space-padded header + data, padded to even length.
// Fields: name/(16) mtime(12) uid(6) gid(6) mode(8) size(10) "`\n"(2).
function arEntry(name: string, data: Buffer): Buffer {
  const header = Buffer.alloc(60, 0x20)
  header.write(`${name}/`, 0, 16, "ascii")
  header.write(`${Math.floor(Date.now() / 1000)}`, 16, 12, "ascii")
  header.write("0", 28, 6, "ascii")
  header.write("0", 34, 6, "ascii")
  header.write("100644", 40, 8, "ascii")
  header.write(`${data.length}`, 48, 10, "ascii")
  header[58] = 0x60
  header[59] = 0x0a

  if (data.length % 2 === 1) {
    return Buffer.concat([header, data, Buffer.from([0x0a])])
  }
  return Buffer.concat([header, data])
}
