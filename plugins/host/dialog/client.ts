import { osCall } from "@tynd/core/client/bridge";

export interface FileFilter {
  /** Display name, e.g. "Images" */
  name: string;
  /** File extensions without the dot, e.g. ["png", "jpg"] */
  extensions: string[];
}

/** Options shared by every file and directory picker. */
export interface PickerOptions {
  title?: string;
  /** Directory the picker opens in. */
  defaultDir?: string;
  /** macOS: allow the user to create directories from the picker. */
  canCreateDirectories?: boolean;
  /** Show hidden files. */
  showHiddenFiles?: boolean;
  /**
   * Make the picker modal to the app's active window (default `true`).
   * Windows only; macOS and Linux pickers are always app-modal.
   */
  parent?: boolean;
  /**
   * Let the app's file APIs reach what the user picks, even outside the
   * configured `security.fs` scope: read-only for the open pickers (a picked
   * directory with its whole subtree), read and write for the file chosen in
   * `saveFile`. App content only: a remote page is refused. `security.fs.deny`
   * still applies. See `capabilities` in `@tynd/core` to grant from the backend.
   */
  grantAccess?: boolean;
}

export interface OpenFileOptions extends PickerOptions {
  filters?: FileFilter[];
}

/** Every OS asks the user before overwriting an existing file. */
export interface SaveFileOptions extends PickerOptions {
  /** Suggested file name */
  defaultName?: string;
  filters?: FileFilter[];
}

/** Options for `dialog.openDirectory` / `dialog.openDirectories`. */
export type OpenDirectoryOptions = PickerOptions;

/** Icon / severity of a message box. */
export type MessageKind = "none" | "info" | "warning" | "error" | "question";

export interface MessageOptions {
  /** Window title (default `"Message"`; macOS alerts have no title bar). */
  title?: string;
  /** Secondary text shown under the message. */
  detail?: string;
  /** Default `"info"`. */
  kind?: MessageKind;
  /** Button labels (up to 16). Without it a single localized "OK" is shown. */
  buttons?: string[];
  /** Index of the button focused when the box opens (default `0`). */
  defaultId?: number;
  /**
   * Index reported when the user dismisses the box (Esc, close button).
   * Defaults to the first button labelled "Cancel" or "No", else `0`.
   */
  cancelId?: number;
  /** Show a checkbox with this label, e.g. "Do not ask again". */
  checkboxLabel?: string;
  /** Initial checkbox state (default `false`). */
  checkboxChecked?: boolean;
  /** Make the box modal to the app's active window (default `true`). */
  parent?: boolean;
}

/** What the user picked in `dialog.message`. */
export interface MessageResult {
  /** Index of the clicked button, or `cancelId` when dismissed. */
  response: number;
  /** Final checkbox state (`false` without `checkboxLabel`). */
  checkboxChecked: boolean;
}

export interface ConfirmOptions {
  title?: string;
  detail?: string;
  kind?: MessageKind;
  /** Make the box modal to the app's active window (default `true`). */
  parent?: boolean;
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
  /** Native multi-select directory picker; resolves to the chosen paths or `null` on cancel. */
  openDirectories(opts?: OpenDirectoryOptions): Promise<string[] | null> {
    return osCall("dialog", "openDirectories", opts ?? null);
  },
  /** Show a native save picker; resolves to the chosen path or `null` on cancel. */
  saveFile(opts?: SaveFileOptions): Promise<string | null> {
    return osCall("dialog", "saveFile", opts ?? null);
  },
  /**
   * Show a native message box with optional custom buttons and checkbox;
   * resolves to the clicked button index and the checkbox state.
   */
  message(message: string, opts?: MessageOptions): Promise<MessageResult> {
    return osCall("dialog", "message", { message, ...opts });
  },
  /** Shorthand for `message(msg, { kind: "warning", ... })`. */
  warn(message: string, opts?: Omit<MessageOptions, "kind">): Promise<MessageResult> {
    return osCall("dialog", "message", { message, ...opts, kind: "warning" });
  },
  /** Shorthand for `message(msg, { kind: "error", ... })`. */
  error(message: string, opts?: Omit<MessageOptions, "kind">): Promise<MessageResult> {
    return osCall("dialog", "message", { message, ...opts, kind: "error" });
  },
  /** Show a localized OK / Cancel box; resolves to `true` when the user presses OK. */
  confirm(message: string, opts?: ConfirmOptions): Promise<boolean> {
    return osCall("dialog", "confirm", { message, ...opts });
  },
};
