import type { ConfirmOptions, MessageOptions, OpenFileOptions, SaveFileOptions } from "../types.js"
import { osCall } from "./_internal.js"

export type { ConfirmOptions, MessageOptions, OpenFileOptions, SaveFileOptions }

export const dialog = {
  openFile(opts?: OpenFileOptions): Promise<string | null> {
    return osCall("dialog", "openFile", opts ?? null)
  },
  openFiles(opts?: OpenFileOptions): Promise<string[] | null> {
    return osCall("dialog", "openFiles", opts ?? null)
  },
  saveFile(opts?: SaveFileOptions): Promise<string | null> {
    return osCall("dialog", "saveFile", opts ?? null)
  },
  message(message: string, opts?: MessageOptions): Promise<void> {
    return osCall("dialog", "message", { message, ...opts })
  },
  confirm(message: string, opts?: ConfirmOptions): Promise<boolean> {
    return osCall("dialog", "confirm", { message, ...opts })
  },
}
