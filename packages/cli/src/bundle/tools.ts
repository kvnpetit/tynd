import { chmodSync, existsSync, mkdirSync, renameSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import type { Arch, Platform } from "../lib/detect.ts"
import { log } from "../lib/logger.ts"

export interface ToolLocation {
  /** Root directory of the installed tool. */
  dir: string
  /** Path to the primary executable (already chmod+x on unix). */
  bin: string
}

export interface ToolSpec {
  name: string
  version: string
  /** null = not supported on this host. */
  url: (platform: Platform, arch: Arch) => string | null
  archive: "zip" | "tar.gz" | "raw"
  /** Path to the main executable inside the extracted archive. For "raw"
   *  downloads, the filename written to disk. */
  binRel: string
}

export async function ensureTool(
  spec: ToolSpec,
  toolsDir: string,
  platform: Platform,
  arch: Arch,
): Promise<ToolLocation> {
  const root = path.join(toolsDir, spec.name, spec.version)
  const bin = path.join(root, spec.binRel)

  if (existsSync(bin)) {
    log.debug(`tool ${spec.name}@${spec.version}: cached at ${root}`)
    return { dir: root, bin }
  }

  const url = spec.url(platform, arch)
  if (!url) {
    throw new Error(`tool ${spec.name}: no download available for ${platform}-${arch}`)
  }

  log.step(`Fetching ${spec.name}@${spec.version}…`)
  log.debug(`tool download: ${url}`)

  mkdirSync(root, { recursive: true })
  const tmpBase = path.join(root, `.download-${Date.now()}`)

  try {
    const res = await fetch(url)
    if (!res.ok) {
      throw new Error(`HTTP ${res.status} for ${url}`)
    }
    const bytes = Buffer.from(await res.arrayBuffer())

    if (spec.archive === "raw") {
      writeFileSync(bin, bytes)
    } else if (spec.archive === "zip") {
      writeFileSync(tmpBase, bytes)
      const AdmZip = (await import("adm-zip")).default
      new AdmZip(tmpBase).extractAllTo(root, true)
      rmSync(tmpBase, { force: true })
    } else {
      writeFileSync(tmpBase, bytes)
      await extractTarGz(tmpBase, root)
      rmSync(tmpBase, { force: true })
    }

    if (existsSync(bin) && platform !== "windows") chmodSync(bin, 0o755)

    if (!existsSync(bin)) {
      throw new Error(
        `tool ${spec.name}: expected ${spec.binRel} after extraction, not found in ${root}`,
      )
    }
  } catch (e) {
    // Wipe partial extraction so the next run re-fetches cleanly.
    rmSync(root, { recursive: true, force: true })
    throw e
  }

  log.success(`${spec.name} cached`)
  return { dir: root, bin }
}

async function extractTarGz(src: string, destDir: string): Promise<void> {
  const { createReadStream } = await import("node:fs")
  const { createGunzip } = await import("node:zlib")
  const tar = (await import("tar-stream")).default.extract()

  await new Promise<void>((resolve, reject) => {
    tar.on("entry", (header, stream, next) => {
      const outPath = path.join(destDir, header.name)
      if (header.type === "directory") {
        mkdirSync(outPath, { recursive: true })
        stream.resume()
        stream.on("end", next)
        return
      }
      mkdirSync(path.dirname(outPath), { recursive: true })
      const chunks: Buffer[] = []
      stream.on("data", (c: Buffer) => chunks.push(c))
      stream.on("end", () => {
        const tmp = `${outPath}.tmp`
        writeFileSync(tmp, Buffer.concat(chunks))
        renameSync(tmp, outPath)
        if (header.mode) {
          try {
            chmodSync(outPath, header.mode & 0o777)
          } catch {
            /* best effort */
          }
        }
        next()
      })
      stream.on("error", next)
    })
    tar.on("finish", resolve)
    tar.on("error", reject)
    createReadStream(src).pipe(createGunzip()).pipe(tar)
  })
}
