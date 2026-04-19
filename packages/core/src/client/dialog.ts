import type { ConfirmOptions, MessageOptions, OpenFileOptions, SaveFileOptions } from "../types.js"
import { osCall } from "./_internal.js"

export type { ConfirmOptions, MessageOptions, OpenFileOptions, SaveFileOptions }

export interface OpenDirectoryOptions {
  title?: string
  defaultDir?: string
}

export const dialog = {
  openFile(opts?: OpenFileOptions): Promise<string | null> {
    return osCall("dialog", "openFile", opts ?? null)
  },
  openFiles(opts?: OpenFileOptions): Promise<string[] | null> {
    return osCall("dialog", "openFiles", opts ?? null)
  },
  /** Native directory picker. Returns the absolute path or `null` on cancel. */
  openDirectory(opts?: OpenDirectoryOptions): Promise<string | null> {
    return osCall("dialog", "openDirectory", opts ?? null)
  },
  saveFile(opts?: SaveFileOptions): Promise<string | null> {
    return osCall("dialog", "saveFile", opts ?? null)
  },
  message(message: string, opts?: MessageOptions): Promise<void> {
    return osCall("dialog", "message", { message, ...opts })
  },
  /** Shorthand for `message(msg, { kind: "warning", ... })`. */
  warn(message: string, opts?: Omit<MessageOptions, "kind">): Promise<void> {
    return osCall("dialog", "message", { message, ...opts, kind: "warning" })
  },
  /** Shorthand for `message(msg, { kind: "error", ... })`. */
  error(message: string, opts?: Omit<MessageOptions, "kind">): Promise<void> {
    return osCall("dialog", "message", { message, ...opts, kind: "error" })
  },
  confirm(message: string, opts?: ConfirmOptions): Promise<boolean> {
    return osCall("dialog", "confirm", { message, ...opts })
  },
}
