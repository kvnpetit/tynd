import { binFetch, binUpload, osCall } from "./_internal.js"

export interface FileStat {
  size: number
  isFile: boolean
  isDir: boolean
  isSymlink: boolean
  mtime: number | null
}

export interface DirEntry {
  name: string
  isFile: boolean
  isDir: boolean
  isSymlink: boolean
}

export const fs = {
  readText(path: string): Promise<string> {
    return osCall("fs", "readText", { path })
  },
  writeText(path: string, content: string, opts?: { createDirs?: boolean }): Promise<void> {
    return osCall("fs", "writeText", { path, content, ...opts })
  },
  exists(path: string): Promise<boolean> {
    return osCall("fs", "exists", { path })
  },
  stat(path: string): Promise<FileStat> {
    return osCall("fs", "stat", { path })
  },
  readDir(path: string): Promise<DirEntry[]> {
    return osCall("fs", "readDir", { path })
  },
  mkdir(path: string, opts?: { recursive?: boolean }): Promise<void> {
    return osCall("fs", "mkdir", { path, ...opts })
  },
  remove(path: string, opts?: { recursive?: boolean }): Promise<void> {
    return osCall("fs", "remove", { path, ...opts })
  },
  rename(from: string, to: string): Promise<void> {
    return osCall("fs", "rename", { from, to })
  },
  copy(from: string, to: string): Promise<void> {
    return osCall("fs", "copy", { from, to })
  },
  readBinary(path: string): Promise<Uint8Array> {
    return binFetch("fs/readBinary", { path })
  },
  writeBinary(
    path: string,
    content: Uint8Array | ArrayBuffer,
    opts?: { createDirs?: boolean },
  ): Promise<void> {
    const bytes = content instanceof ArrayBuffer ? new Uint8Array(content) : content
    const query: Record<string, string> = { path }
    if (opts?.createDirs) query["createDirs"] = "1"
    return binUpload("fs/writeBinary", query, bytes)
  },
}
