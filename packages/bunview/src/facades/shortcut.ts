import type { WebviewHost } from "../host";

export function shortcutFacade(host: WebviewHost | undefined) {
  return {
    register:    (id: string, accelerator: string) => host?.shortcutRegister(id, accelerator),
    unregister:  (id: string)                      => host?.shortcutUnregister(id),
    onTriggered: (cb: (id: string) => void)        => { host?.on("shortcutTriggered", cb); },
  };
}
