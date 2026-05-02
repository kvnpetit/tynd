import { osCall } from "./_internal";

export interface SecurityScope {
  /** Glob patterns that are allowed. Empty = open (unless `defaultDeny`). */
  allow?: string[];
  /** Glob patterns that are always denied (takes precedence over allow). */
  deny?: string[];
}

export interface SecurityPolicy {
  /** Rules applied to every `fs.*` method (including binary reads / writes). */
  fs?: SecurityScope;
  /** Rules applied to `http.request` / `http.download` URLs. */
  http?: SecurityScope;
  /**
   * When true, an empty `allow` list denies every call of that scope.
   * Default is `false` — i.e. no configuration means open.
   */
  defaultDeny?: boolean;
}

/**
 * Which scope a persisted-grant call targets. `"fs"` covers paths
 * granted via folder pickers; `"http"` covers URLs granted via in-app
 * permission prompts. Defaults to `"fs"` everywhere on the JS side to
 * match the most common picker-then-persist flow.
 */
export type GrantKind = "fs" | "http";

export const security = {
  /**
   * Install an allow/deny policy. Patterns use `*` (no separator) and
   * `**` (any run). FS paths normalized to forward slashes + lowercased
   * on Windows. Deny always beats allow.
   *
   * Persisted grants (see `persistGrant` below) are merged into the
   * effective allow-list automatically — calling `configure` does NOT
   * wipe folders the user previously granted at runtime.
   *
   * @example
   *   await security.configure({
   *     fs: { allow: ["${home}/Documents/**"], deny: ["**\/.ssh/**"] },
   *     http: { allow: ["https://api.myapp.com/**"] },
   *     defaultDeny: true,
   *   })
   */
  configure(policy: SecurityPolicy): Promise<void> {
    return osCall("security", "configure", policy);
  },
  isFsAllowed(path: string): Promise<boolean> {
    return osCall("security", "isFsAllowed", { path });
  },
  isHttpAllowed(url: string): Promise<boolean> {
    return osCall("security", "isHttpAllowed", { url });
  },

  // -------------------------------------------------------------------
  // Persisted grants — folded from the former `@tynd/plugin-persisted-scope`.
  //
  // Pattern: after a folder picker, call `persistGrant(folder)` so the
  // user doesn't have to re-pick on the next launch. The host extends
  // the live ACL AND mirrors the entry into a JSON file under the app's
  // data dir; the next process start auto-restores it (lazily, on the
  // first allow-mutation or `configure(...)` call).
  // -------------------------------------------------------------------

  /**
   * Persist a path so it's auto-restored on the next launch. Also
   * extends the live policy via `addFsAllow` / `addHttpAllow` so the
   * very next call sees the grant. Returns `true` if newly persisted.
   */
  persistGrant(path: string, kind: GrantKind = "fs"): Promise<boolean> {
    return osCall("security", "persistGrant", { path, kind }) as Promise<boolean>;
  },
  /**
   * Remove a path from the persisted JSON. Does NOT remove it from the
   * current allow-list — call `removeFsAllow` / `removeHttpAllow` (TBD)
   * separately if that's what you want.
   */
  revokePersisted(path: string, kind: GrantKind = "fs"): Promise<boolean> {
    return osCall("security", "revokePersisted", { path, kind }) as Promise<boolean>;
  },
  /** Snapshot of every currently-persisted entry of the requested kind. */
  listPersisted(kind: GrantKind = "fs"): Promise<string[]> {
    return osCall("security", "listPersisted", { kind }) as Promise<string[]>;
  },
  /** Membership check — no I/O, no policy mutation. */
  hasPersisted(path: string, kind: GrantKind = "fs"): Promise<boolean> {
    return osCall("security", "hasPersisted", { path, kind }) as Promise<boolean>;
  },
  /** Drop every persisted entry of a kind. Live policy is untouched. */
  clearPersisted(kind: GrantKind = "fs"): Promise<void> {
    return osCall("security", "clearPersisted", { path: "", kind });
  },
};
