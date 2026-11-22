import { osCall } from "@tynd/core/client/bridge";

export interface FileFilter {
  /** Display name, e.g. "Images" */
  name: string;
  /** File extensions without the dot, e.g. ["png", "jpg"] */
  extensions: string[];
}

export interface OpenFileOptions {
  title?: string;
  defaultDir?: string;
  filters?: FileFilter[];
  /** macOS: allow the user to create directories from the picker. */
  canCreateDirectories?: boolean;
}

export interface SaveFileOptions {
  title?: string;
  /** Suggested file name */
  defaultName?: string;
  defaultDir?: string;
  filters?: FileFilter[];
  /** macOS: allow the user to create directories from the picker. */
  canCreateDirectories?: boolean;
}

export interface MessageOptions {
  title?: string;
  kind?: "info" | "warning" | "error";
}

export interface ConfirmOptions {
  title?: string;
}

/** Options for `dialog.openDirectory`. */
export interface OpenDirectoryOptions {
  title?: string;
  defaultDir?: string;
  /** macOS: allow the user to create directories from the picker. */
  canCreateDirectories?: boolean;
}

/** Frontend bindings for the host's `dialog` API (native open/save/message/confirm pickers). */
export const dialog = {
  /** Show a native single-file picker; resolves to the chosen path or `null` on cancel. */
  openFile(opts?: OpenFileOptions): Promise<string | null> {
    return osCall("dialog", "openFile", opts ?? null);
  },
  /** Show a native multi-file picker; resolves to the chosen paths or `null` on cancel. */
  openFiles(opts?: OpenFileOptions): Promise<string[] | null> {
    return osCall("dialog", "openFiles", opts ?? null);
  },
  /** Native directory picker. Returns the absolute path or `null` on cancel. */
  openDirectory(opts?: OpenDirectoryOptions): Promise<string | null> {
    return osCall("dialog", "openDirectory", opts ?? null);
  },
  /** Show a native save picker; resolves to the chosen path or `null` on cancel. */
  saveFile(opts?: SaveFileOptions): Promise<string | null> {
    return osCall("dialog", "saveFile", opts ?? null);
  },
  /** Show a modal info message dialog; resolves once the user dismisses it. */
  message(message: string, opts?: MessageOptions): Promise<void> {
    return osCall("dialog", "message", { message, ...opts });
  },
  /** Shorthand for `message(msg, { kind: "warning", ... })`. */
  warn(message: string, opts?: Omit<MessageOptions, "kind">): Promise<void> {
    return osCall("dialog", "message", { message, ...opts, kind: "warning" });
  },
  /** Shorthand for `message(msg, { kind: "error", ... })`. */
  error(message: string, opts?: Omit<MessageOptions, "kind">): Promise<void> {
    return osCall("dialog", "message", { message, ...opts, kind: "error" });
  },
  /** Show a modal confirm dialog; resolves to `true` when the user accepts. */
  confirm(message: string, opts?: ConfirmOptions): Promise<boolean> {
    return osCall("dialog", "confirm", { message, ...opts });
  },
};
