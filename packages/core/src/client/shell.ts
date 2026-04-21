import { osCall } from "./_internal.js"

export const shell = {
  openExternal(url: string): Promise<void> {
    return osCall("shell", "openExternal", url)
  },
  openPath(path: string): Promise<void> {
    return osCall("shell", "openPath", path)
  },
  /**
   * Open the OS file manager with `path` selected. Works on Windows
   * (explorer /select), macOS (Finder -R). On Linux, opens the parent
   * directory — no standard "select" gesture exists across file managers.
   */
  revealInFolder(path: string): Promise<void> {
    return osCall("shell", "revealInFolder", path)
  },
}
