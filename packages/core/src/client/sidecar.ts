import { osCall } from "./_internal.js"

export const sidecar = {
  path(name: string): Promise<string> {
    return osCall("sidecar", "path", { name }) as Promise<string>
  },
  list(): Promise<Array<{ name: string; path: string }>> {
    return osCall("sidecar", "list")
  },
}
