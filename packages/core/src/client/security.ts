import { osCall } from "./_internal.js"

export interface SecurityScope {
  /** Glob patterns that are allowed. Empty = open (unless `defaultDeny`). */
  allow?: string[]
  /** Glob patterns that are always denied (takes precedence over allow). */
  deny?: string[]
}

export interface SecurityPolicy {
  /** Rules applied to every `fs.*` method (including binary reads / writes). */
  fs?: SecurityScope
  /** Rules applied to `http.request` / `http.download` URLs. */
  http?: SecurityScope
  /**
   * When true, an empty `allow` list denies every call of that scope.
   * Default is `false` — i.e. no configuration means open.
   */
  defaultDeny?: boolean
}

export const security = {
  /**
   * Install an allow/deny policy. Patterns use `*` (no separator) and
   * `**` (any run). FS paths normalized to forward slashes + lowercased
   * on Windows. Deny always beats allow.
   *
   * @example
   *   await security.configure({
   *     fs: { allow: ["${home}/Documents/**"], deny: ["**\/.ssh/**"] },
   *     http: { allow: ["https://api.myapp.com/**"] },
   *     defaultDeny: true,
   *   })
   */
  configure(policy: SecurityPolicy): Promise<void> {
    return osCall("security", "configure", policy)
  },
  isFsAllowed(path: string): Promise<boolean> {
    return osCall("security", "isFsAllowed", { path })
  },
  isHttpAllowed(url: string): Promise<boolean> {
    return osCall("security", "isHttpAllowed", { url })
  },
}
