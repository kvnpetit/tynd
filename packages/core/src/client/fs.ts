import { base64ToBytes, binFetch, binUpload, bytesToBase64, osCall } from "./_internal.js"

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

export interface OpenOptions {
  /** Default true. */
  read?: boolean
  write?: boolean
  append?: boolean
  create?: boolean
  truncate?: boolean
}

export type SeekFrom = "start" | "current" | "end"

export interface FileHandle {
  id: number
  /** Move the cursor; returns the new absolute position. */
  seek(offset: number, from?: SeekFrom): Promise<number>
  /** Read up to `length` bytes at the current position. */
  read(length: number): Promise<{ bytes: Uint8Array; read: number }>
  /** Write `content` at the current position; returns bytes written. */
  write(content: Uint8Array | ArrayBuffer): Promise<number>
  /** Release the OS handle. Calling again is a no-op. */
  close(): Promise<boolean>
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
  /** Recursive directory copy — file contents only, symlinks skipped. */
  copyDir(from: string, to: string): Promise<void> {
    return osCall("fs", "copyDir", { from, to })
  },

  /** Create a symbolic link at `link` pointing to `target`. */
  symlink(target: string, link: string): Promise<void> {
    return osCall("fs", "symlink", { target, link })
  },
  /** Read the target of a symbolic link. */
  readlink(path: string): Promise<string> {
    return osCall("fs", "readlink", { path })
  },
  /** Create a hard link at `link` pointing to `original`. */
  hardlink(original: string, link: string): Promise<void> {
    return osCall("fs", "hardlink", { original, link })
  },

  /**
   * Open a file handle for stateful seek / read / write. Returns a
   * `FileHandle` that must be `close()`d. Binary payloads travel base64
   * over JSON — for multi-MB I/O prefer `readBinary` / `writeBinary`.
   */
  async open(path: string, options?: OpenOptions): Promise<FileHandle> {
    const { id } = await osCall<{ id: number }>("fs", "open", { path, ...options })
    return {
      id,
      async seek(offset, from = "start") {
        const { position } = await osCall<{ position: number }>("fs", "seek", {
          id,
          offset,
          from,
        })
        return position
      },
      async read(length) {
        const res = await osCall<{ bytes: string; read: number }>("fs", "read", { id, length })
        return { bytes: base64ToBytes(res.bytes), read: res.read }
      },
      async write(content) {
        const bytes = content instanceof ArrayBuffer ? new Uint8Array(content) : content
        const { written } = await osCall<{ written: number }>("fs", "write", {
          id,
          bytes: bytesToBase64(bytes),
        })
        return written
      },
      close() {
        return osCall("fs", "close", { id })
      },
    }
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
