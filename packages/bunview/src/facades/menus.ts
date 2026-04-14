import type { WebviewHost } from "../host";
import type { MenuItem, TrayMenuItem } from "../types";

export function trayFacade(host: WebviewHost | undefined) {
  return {
    create:      (tooltip?: string, icon?: string) => host?.trayCreate(tooltip, icon),
    setMenu:     (items: TrayMenuItem[])           => host?.traySetMenu(items),
    remove:      ()                                => host?.trayRemove(),
    onClick:     (cb: () => void)                  => { host?.on("trayClick", cb); },
    onMenuClick: (cb: (id: string) => void)        => { host?.on("trayMenuItemClick", cb); },
  };
}

export function menuFacade(host: WebviewHost | undefined) {
  return {
    set:     (items: MenuItem[])        => host?.menuSet(items),
    remove:  ()                         => host?.menuRemove(),
    onClick: (cb: (id: string) => void) => { host?.on("menuItemClick", cb); },
  };
}

export function contextMenuFacade(host: WebviewHost | undefined) {
  return {
    show:    (items: MenuItem[], x?: number, y?: number) => host?.contextMenuShow(items, x, y),
    onClick: (cb: (id: string) => void)                  => { host?.on("contextMenuItemClick", cb); },
  };
}
