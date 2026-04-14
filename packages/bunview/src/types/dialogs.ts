export interface FileFilter {
  name: string;
  extensions: string[];   // e.g. ["png", "jpg"]
}

export interface OpenFileOptions {
  filters?: FileFilter[];
  multiple?: boolean;
  defaultPath?: string;
  title?: string;
}

export interface SaveFileOptions {
  filters?: FileFilter[];
  defaultPath?: string;
  title?: string;
}

export interface OpenDirectoryOptions {
  defaultPath?: string;
  title?: string;
}

export interface MessageDialogOptions {
  title?: string;
  message: string;
  type?: "alert" | "confirm" | "input";
  defaultValue?: string;
}
