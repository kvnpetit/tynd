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

export interface FsChangeEvent {
  /** Numeric id of the watcher that produced this event. */
  id: number
  kind: "create" | "modify" | "delete" | "rename" | "error" | "other"
  /** Absolute path of the changed entry (or empty on error). */
  path?: string
  /** Set when kind === "error". */
  error?: string
}

export interface WatchHandle {
  id: number
  /** Stop receiving events and release the OS watcher. */
  unwatch(): Promise<boolean>
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
  /** Move to the OS recycle bin / Trash. Reversible unlike `remove()`. */
  trash(path: string): Promise<void> {
    return osCall("fs", "trash", { path })
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
  /**
   * Watch a path for changes (native: ReadDirectoryChangesW / FSEvents /
   * inotify). The handler fires for each file-system event matching the id
   * returned by the host — multiple watchers are independent.
   */
  async watch(
    path: string,
    opts: { recursive?: boolean } | undefined,
    handler: (event: FsChangeEvent) => void,
  ): Promise<WatchHandle> {
    const { id } = await osCall<{ id: number }>("fs", "watch", { path, ...opts })
    const off = window.__tynd__.os_on("fs:change", (raw) => {
      const event = raw as FsChangeEvent
      if (event.id === id) handler(event)
    })
    return {
      id,
      async unwatch() {
        off()
        return osCall<boolean>("fs", "unwatch", { id })
      },
    }
  },
}
