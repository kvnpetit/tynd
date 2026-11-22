(function () {
  "use strict";
  // Declarative context menus. On right-click, the closest element carrying
  // `data-tynd-contextmenu` decides what happens:
  //   "<menuId>" -> open the native menu registered with
  //                 menu.registerContextMenu(menuId, items); the element's
  //                 `data-tynd-contextmenu-data` rides on the click event.
  //   "none"     -> suppress the webview context menu (put it on <html> to
  //                 disable it app-wide).
  //   "default"  -> keep the webview context menu below a "none" ancestor.
  if (window.__tynd_contextmenu__) return;
  window.__tynd_contextmenu__ = true;

  document.addEventListener("contextmenu", function (event) {
    if (event.defaultPrevented) return;
    var target = event.target;
    if (!target || typeof target.closest !== "function") return;
    var element = target.closest("[data-tynd-contextmenu]");
    if (!element) return;
    var menuId = element.getAttribute("data-tynd-contextmenu");
    if (!menuId || menuId === "default") return;
    event.preventDefault();
    if (menuId === "none") return;
    var tynd = window.__tynd__;
    if (!tynd || typeof tynd.os_call !== "function") return;
    // No x / y: the native menu opens at the cursor, which stays correct
    // under page zoom and inside child webviews.
    tynd
      .os_call("menu", "popupContextMenu", {
        menu: menuId,
        data: element.getAttribute("data-tynd-contextmenu-data"),
      })
      .catch(function (error) {
        if (window.__tynd_dev__) {
          // biome-ignore lint/suspicious/noConsole: dev-only diagnostic for a failed popup
          console.error("[tynd] context menu '" + menuId + "':", error);
        }
      });
  });
})();
