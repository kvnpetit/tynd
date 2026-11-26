import { osCall } from "./_internal";

export type { SecurityPolicy, SecurityScope } from "../types";

/**
 * Read-only view of the host security policy.
 *
 * Configure policy from the trusted Bun backend with
 * `app.start({ security: { ... } })`. Renderer code intentionally cannot
 * mutate policy.
 */
export const security = {
  isFsAllowed(path: string): Promise<boolean> {
    return osCall("security", "isFsAllowed", { path });
  },
  isHttpAllowed(url: string): Promise<boolean> {
    return osCall("security", "isHttpAllowed", { url });
  },
};
