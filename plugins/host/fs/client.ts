import {
  base64ToBytes,
  binFetch,
  binUpload,
  bytesToBase64,
  osCall,
  osOn,
} from "@tynd/core/client/bridge";

/** Metadata returned by `fs.stat`. */
export interface FileStat {
  size: number;
  isFile: boolean;
  isDir: boolean;
  isSymlink: boolean;
  mtime: number | null;
}

/** Single entry yielded by `fs.readDir`. */
export interface DirEntry {
  name: string;
  isFile: boolean;
  isDir: boolean;
  isSymlink: boolean;
}

/** Event emitted by `fs.watch`. */
export interface FsChangeEvent {
  /** Numeric id of the watcher that produced this event. */
  id: number;
  kind: "create" | "modify" | "delete" | "rename" | "error" | "other";
  /** Absolute path of the changed entry (or empty on error). */
  path?: string;
  /** Set when kind === "error". */
  error?: string;
}

/** Handle returned by `fs.watch`; call `unwatch()` to release the OS watcher. */
export interface WatchHandle {
  id: number;
  /** Stop receiving events and release the OS watcher. */
  unwatch(): Promise<boolean>;
}

/** Mode flags for `fs.open`. */
export interface OpenOptions {
  /** Default true. */
  read?: boolean;
  write?: boolean;
  append?: boolean;
  create?: boolean;
  truncate?: boolean;
}

/** Reference point for `FileHandle.seek`. */
export type SeekFrom = "start" | "current" | "end";

/** Stateful file handle returned by `fs.open`. */
export interface FileHandle extends AsyncDisposable {
  id: number;
  /** Move the cursor; returns the new absolute position. */
  seek(offset: number, from?: SeekFrom): Promise<number>;
  /** Read up to `length` bytes at the current position. */
  read(length: number): Promise<{ bytes: Uint8Array; read: number }>;
  /** Write `content` at the current position; returns bytes written. */
  write(content: Uint8Array | ArrayBuffer): Promise<number>;
  /**
   * Release the OS handle. Idempotent — calling again is a no-op.
   * Prefer the TC39 `await using` syntax (see `[Symbol.asyncDispose]`)
   * for automatic cleanup at scope exit.
   */
  close(): Promise<boolean>;
  /**
   * Auto-close hook for the TC39 `await using` syntax. Idempotent — safe
   * to call after an explicit `close()`. Pairs with `Symbol.asyncDispose`
   * (Node 22+, Bun 1.2+, TS 5.2+).
   * @example
   *   {
   *     await using h = await fs.open("/tmp/log.txt", { write: true });
   *     await h.write(new TextEncoder().encode("hello\n"));
   *     // h.close() runs automatically when the block exits — even on throw.
   *   }
   */
  [Symbol.asyncDispose](): Promise<void>;
}

/** Frontend bindings for the host's `fs` API (text/binary IO, watchers, file handles). */
export const fs = {
  /** Read the entire file as a UTF-8 string. */
  readText(path: string): Promise<string> {
    return osCall("fs", "readText", { path });
  },
  /** Write a UTF-8 string to the file, optionally creating missing parent dirs. */
  writeText(path: string, content: string, opts?: { createDirs?: boolean }): Promise<void> {
    return osCall("fs", "writeText", { path, content, ...opts });
  },
  /** Whether the path exists (file, dir, or symlink). */
  exists(path: string): Promise<boolean> {
    return osCall("fs", "exists", { path });
  },
  /** Read filesystem metadata for the path. */
  stat(path: string): Promise<FileStat> {
    return osCall("fs", "stat", { path });
  },
  /** List immediate children of a directory. */
  readDir(path: string): Promise<DirEntry[]> {
    return osCall("fs", "readDir", { path });
  },
  /** Create a directory (optionally recursively). */
  mkdir(path: string, opts?: { recursive?: boolean }): Promise<void> {
    return osCall("fs", "mkdir", { path, ...opts });
  },
  /** Delete a file or directory (set `recursive` for non-empty dirs). */
  remove(path: string, opts?: { recursive?: boolean }): Promise<void> {
    return osCall("fs", "remove", { path, ...opts });
  },
  /** Rename or move an entry. */
  rename(from: string, to: string): Promise<void> {
    return osCall("fs", "rename", { from, to });
  },
  /** Copy a file. */
  copy(from: string, to: string): Promise<void> {
    return osCall("fs", "copy", { from, to });
  },
  /** Move to the OS recycle bin / Trash. Reversible unlike `remove()`. */
  trash(path: string): Promise<void> {
    return osCall("fs", "trash", { path });
  },
  /** Recursive directory copy — file contents only, symlinks skipped. */
  copyDir(from: string, to: string): Promise<void> {
    return osCall("fs", "copyDir", { from, to });
  },

  /** Create a symbolic link at `link` pointing to `target`. */
  symlink(target: string, link: string): Promise<void> {
    return osCall("fs", "symlink", { target, link });
  },
  /** Read the target of a symbolic link. */
  readlink(path: string): Promise<string> {
    return osCall("fs", "readlink", { path });
  },
  /** Create a hard link at `link` pointing to `original`. */
  hardlink(original: string, link: string): Promise<void> {
    return osCall("fs", "hardlink", { original, link });
  },

  /**
   * Open a file handle for stateful seek / read / write. Returns a
   * `FileHandle` that must be `close()`d (or, preferably, scoped with the
   * TC39 `await using` syntax — see `FileHandle[Symbol.asyncDispose]`).
   * Binary payloads travel base64 over JSON — for multi-MB I/O prefer
   * `readBinary` / `writeBinary`.
   * @example
   *   await using h = await fs.open("/tmp/log.txt", { write: true });
   *   await h.write(new TextEncoder().encode("hello\n"));
   */
  async open(path: string, options?: OpenOptions): Promise<FileHandle> {
    const { id } = await osCall<{ id: number }>("fs", "open", { path, ...options });
    // `closed` flag makes both `close()` and the asyncDispose hook
    // idempotent — host already returns false on a stale id, but
    // short-circuiting here saves a round-trip and silences the
    // misleading `false` return on the second call.
    let closed = false;
    const handle: FileHandle = {
      id,
      async seek(offset, from = "start") {
        const { position } = await osCall<{ position: number }>("fs", "seek", {
          id,
          offset,
          from,
        });
        return position;
      },
      async read(length) {
        const res = await osCall<{ bytes: string; read: number }>("fs", "read", { id, length });
        return { bytes: base64ToBytes(res.bytes), read: res.read };
      },
      async write(content) {
        const bytes = content instanceof ArrayBuffer ? new Uint8Array(content) : content;
        const { written } = await osCall<{ written: number }>("fs", "write", {
          id,
          bytes: bytesToBase64(bytes),
        });
        return written;
      },
      async close() {
        if (closed) return false;
        closed = true;
        return osCall<boolean>("fs", "close", { id });
      },
      async [Symbol.asyncDispose]() {
        if (closed) return;
        closed = true;
        await osCall("fs", "close", { id });
      },
    };
    return handle;
  },
  /** Read a file's bytes via the binary side-channel (avoids base64 over JSON). */
  readBinary(path: string): Promise<Uint8Array> {
    return binFetch("fs/readBinary", { path });
  },
  /** Write bytes to a file via the binary side-channel. */
  writeBinary(
    path: string,
    content: Uint8Array | ArrayBuffer,
    opts?: { createDirs?: boolean },
  ): Promise<void> {
    const query: Record<string, string> = { path };
    if (opts?.createDirs) query.createDirs = "1";
    return binUpload("fs/writeBinary", query, content);
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
    const { id } = await osCall<{ id: number }>("fs", "watch", { path, ...opts });
    const off = osOn("fs:change", (raw) => {
      const event = raw as FsChangeEvent;
      if (event.id === id) handler(event);
    });
    return {
      id,
      async unwatch() {
        off();
        return osCall<boolean>("fs", "unwatch", { id });
      },
    };
  },
};
