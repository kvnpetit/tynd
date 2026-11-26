/**
 * Runtime scope grants, from the trusted Bun backend only.
 *
 * `app.start({ security })` sets the filesystem and HTTP scopes at startup.
 * These calls add named paths and URL patterns afterwards, typically right
 * after the user picked a file or folder. A grant never overrides a
 * `security.fs.deny` / `security.http.deny` rule, and renderers and native
 * plugins cannot call any of these methods.
 */

import { hostCall } from "./host";

/** Options for {@link capabilities.allowPath}. */
export interface AllowPathOptions {
  /** For a directory: cover its whole subtree (default: direct entries only). */
  recursive?: boolean;
  /** Also allow writes: create, modify, rename, delete (default: read-only). */
  write?: boolean;
}

/** One path grant as the host stores it. */
export interface PathGrant {
  /** The path resolved by the OS when the grant was made (symlinks followed). */
  path: string;
  /** The grant names a directory. */
  directory: boolean;
  recursive: boolean;
  write: boolean;
}

/** Every live grant. */
export interface ScopeGrants {
  paths: PathGrant[];
  urls: string[];
}

export const capabilities = {
  /**
   * Let the app's file APIs reach `path` (a file, a directory, or a file the
   * app is about to create), even outside the default roots and outside a
   * configured `security.fs.allow` list. The path is resolved by the OS now:
   * a later symlink swap cannot redirect the grant. Granting the same path
   * again replaces the previous grant.
   * @example
   *   await capabilities.allowPath("/Volumes/Photos", { recursive: true });
   */
  allowPath(path: string, options: AllowPathOptions = {}): Promise<PathGrant> {
    return hostCall("security", "allowPath", { path, ...options });
  },

  /** Remove the grant made for `path`. Resolves `false` when there was none. */
  revokePath(path: string): Promise<boolean> {
    return hostCall("security", "revokePath", { path });
  },

  /**
   * Add an `http(s)://` URL glob (same syntax as `security.http.allow`) to a
   * configured HTTP allow-list: `fetch`, downloads and main-frame navigation.
   * The pattern must name a host; only a leading `*.` wildcard is accepted
   * there. Without a configured `security.http` policy every URL is already
   * allowed for requests, and remote navigation stays refused.
   * @example
   *   await capabilities.allowUrl("https://*.example.com/**");
   */
  allowUrl(pattern: string): Promise<void> {
    return hostCall("security", "allowUrl", { pattern });
  },

  /** Remove a pattern added by {@link capabilities.allowUrl}. */
  revokeUrl(pattern: string): Promise<boolean> {
    return hostCall("security", "revokeUrl", { pattern });
  },

  /** Every live grant (persisted ones included with `security.persistGrants`). */
  list(): Promise<ScopeGrants> {
    return hostCall("security", "listGrants", null);
  },
};
