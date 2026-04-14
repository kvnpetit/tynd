import type { WebviewHost } from "../host";
import type { OpenFileOptions, SaveFileOptions, OpenDirectoryOptions } from "../types";

export function dialogFacade(host: WebviewHost | undefined) {
  return {
    open:      (opts?: OpenFileOptions)      => host?.openFile(opts)       ?? Promise.resolve(null),
    save:      (opts?: SaveFileOptions)      => host?.saveFile(opts)       ?? Promise.resolve(null),
    directory: (opts?: OpenDirectoryOptions) => host?.openDirectory(opts)  ?? Promise.resolve(null),
    message:   (message: string, title?: string) =>
      host?.messageDialog({ message, title, dialogType: "alert" }) ?? Promise.resolve(null),
    confirm:   (message: string, title?: string) =>
      host?.messageDialog({ message, title, dialogType: "confirm" }) ?? Promise.resolve(false),
    input:     (message: string, title?: string, defaultValue?: string) =>
      host?.messageDialog({ message, title, dialogType: "input", defaultValue }) ?? Promise.resolve(null),
  };
}
