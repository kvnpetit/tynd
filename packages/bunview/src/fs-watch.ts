import { watch, type FSWatcher, statSync, existsSync } from "fs";
import path from "path";

export type FsEventType = "add" | "change" | "unlink" | "addDir" | "unlinkDir";

export interface FsEvent {
  type: FsEventType;
  path: string;
}

export interface WatchOptions {
  /** Default: false. */
  recursive?: boolean;
  /** Glob substrings or RegExp tested against absolute paths. */
  ignored?: (string | RegExp)[];
  /** Default: 25. */
  debounceMs?: number;
}

/**
 * Recursive mode is native on Windows + macOS and falls back to manual
 * per-directory watchers on Linux (inotify is not recursive).
 */
export function watchPath(
  target: string,
  handler: (event: FsEvent) => void,
  opts: WatchOptions = {},
): () => void {
  const recursive = !!opts.recursive;
  const debounceMs = opts.debounceMs ?? 25;
  const ignored    = opts.ignored ?? [];

  const isIgnored = (p: string): boolean =>
    ignored.some((pat) => (typeof pat === "string" ? p.includes(pat) : pat.test(p)));

  // fs.watch reports "rename" or "change" — not lifecycle. Track last-seen
  // metadata to classify events as add / change / unlink / addDir / unlinkDir.
  const seen = new Map<string, number>();
  const seenDirs = new Set<string>();
  const watchers: FSWatcher[] = [];
  const pending = new Map<string, ReturnType<typeof setTimeout>>();

  const classify = (fullPath: string) => {
    const exists = existsSync(fullPath);
    if (!exists) {
      if (seenDirs.delete(fullPath)) { handler({ type: "unlinkDir", path: fullPath }); return; }
      if (seen.delete(fullPath))     { handler({ type: "unlink",    path: fullPath }); return; }
      return;
    }
    const st = statSync(fullPath);
    if (st.isDirectory()) {
      if (!seenDirs.has(fullPath)) { seenDirs.add(fullPath); handler({ type: "addDir", path: fullPath }); }
      return;
    }
    const prev = seen.get(fullPath);
    seen.set(fullPath, st.mtimeMs);
    handler({ type: prev == null ? "add" : "change", path: fullPath });
  };

  const schedule = (fullPath: string) => {
    if (isIgnored(fullPath)) return;
    const prior = pending.get(fullPath);
    if (prior) clearTimeout(prior);
    pending.set(fullPath, setTimeout(() => {
      pending.delete(fullPath);
      classify(fullPath);
    }, debounceMs));
  };

  // Seed state so subsequent events classify without a spurious initial fire.
  if (existsSync(target)) {
    const st = statSync(target);
    if (st.isDirectory()) seenDirs.add(target);
    else                  seen.set(target, st.mtimeMs);
  }

  // Native recursive watch works on Windows + macOS; Linux requires emulation.
  const nativeRecursive = recursive && (process.platform === "win32" || process.platform === "darwin");

  const addWatcher = (dir: string, rec: boolean) => {
    try {
      const w = watch(dir, { recursive: rec }, (_evt, filename) => {
        if (!filename) return;
        const full = path.join(dir, filename);
        schedule(full);
      });
      watchers.push(w);
    } catch { /* transient dir removal */ }
  };

  if (nativeRecursive) {
    addWatcher(target, true);
  } else if (recursive && existsSync(target) && statSync(target).isDirectory()) {
    const walk = (dir: string) => {
      if (isIgnored(dir)) return;
      addWatcher(dir, false);
      try {
        const { readdirSync } = require("fs") as typeof import("fs");
        for (const name of readdirSync(dir)) {
          const full = path.join(dir, name);
          if (statSync(full).isDirectory()) walk(full);
        }
      } catch { /* race on removed dir */ }
    };
    walk(target);
  } else {
    addWatcher(target, false);
  }

  return () => {
    for (const t of pending.values()) clearTimeout(t);
    pending.clear();
    for (const w of watchers) { try { w.close(); } catch { /* already closed */ } }
  };
}
