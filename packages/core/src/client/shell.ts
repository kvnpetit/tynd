import { osCall } from "./_internal.js"

export const shell = {
  openExternal(url: string): Promise<void> {
    return osCall("shell", "openExternal", url)
  },
  openPath(path: string): Promise<void> {
    return osCall("shell", "openPath", path)
  },
}
