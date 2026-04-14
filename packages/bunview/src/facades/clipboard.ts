import type { WebviewHost } from "../host";

export function clipboardFacade(host: WebviewHost | undefined) {
  return {
    read:      ()                              => host?.clipboardRead() ?? Promise.resolve(""),
    write:     (text: string)                  => host?.clipboardWrite(text),
    writeHtml: (html: string, text?: string)   => host?.clipboardWriteHtml(html, text),
    clear:     ()                              => host?.clipboardClear(),
  };
}
