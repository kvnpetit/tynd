import { osCall } from "@tynd/core/client/bridge";

export const sidecar = {
  path(name: string): Promise<string> {
    return osCall("sidecar", "path", { name });
  },
  list(): Promise<Array<{ name: string; path: string }>> {
    return osCall("sidecar", "list");
  },
};
