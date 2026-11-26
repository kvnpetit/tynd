// Frameless-window helpers, injected into every webview after the IPC shim.
//
// - `data-tynd-drag` on an element drags the window from it (and its
//   descendants); double-click toggles maximize. Controls inside it (button,
//   input, select, textarea, a, [contenteditable], [role=button]) and any
//   element under `data-tynd-no-drag` stay clickable.
// - `data-tynd-resize="n|ne|e|se|s|sw|w|nw"` starts a native resize from
//   that edge / corner.
// - A frameless, resizable, restored window also resizes from a 5 px strip
//   along the page border (Windows / Linux; macOS handles it natively).
// - `data-tynd-drag-file="<absolute path>"` on a draggable element (images,
//   links, or `draggable="true"`) drags that file out to other apps; separate
//   several paths with newlines. Paths go through the `fs` policy.
(function () {
  "use strict";
  if (window.__tynd_frame__) return;
  window.__tynd_frame__ = true;

  var EDGE = 5;
  var DIRECTIONS = {
    n: "north",
    ne: "northEast",
    e: "east",
    se: "southEast",
    s: "south",
    sw: "southWest",
    w: "west",
    nw: "northWest",
  };
  var CURSORS = {
    n: "ns-resize",
    s: "ns-resize",
    e: "ew-resize",
    w: "ew-resize",
    ne: "nesw-resize",
    sw: "nesw-resize",
    nw: "nwse-resize",
    se: "nwse-resize",
  };
  var CONTROLS = "button,input,select,textarea,a,[contenteditable],[role=button]";

  function call(method, args) {
    var bridge = window.__tynd__;
    if (!bridge || typeof bridge.os_call !== "function") return Promise.resolve(null);
    return bridge.os_call("window", method, args || {}).catch(function () {
      return null;
    });
  }

  // `edgeResizable` is cached for one second and dropped on every
  // maximize / fullscreen flip so the border strip follows the window state.
  var edgeResizable = false;
  var checkedAt = 0;
  var checking = false;
  function refreshFrameState() {
    if (checking || Date.now() - checkedAt < 1000) return;
    checking = true;
    call("frameState").then(function (state) {
      edgeResizable = !!(state && state.edgeResizable);
      checkedAt = Date.now();
      checking = false;
    });
  }
  function invalidate() {
    checkedAt = 0;
    edgeResizable = false;
  }
  if (window.__tynd__ && typeof window.__tynd__.os_on === "function") {
    ["maximized", "unmaximized", "fullscreen", "unfullscreen"].forEach(function (name) {
      window.__tynd__.os_on("window:" + name, invalidate);
    });
  }

  function edgeAt(x, y) {
    var w = window.innerWidth;
    var h = window.innerHeight;
    var v = y < EDGE ? "n" : y >= h - EDGE ? "s" : "";
    var hz = x < EDGE ? "w" : x >= w - EDGE ? "e" : "";
    return v + hz;
  }

  // Cursor feedback goes through the CSSOM (not a style attribute or a
  // <style> tag) so a strict page CSP does not block it.
  var cursorTarget = null;
  var cursorPrevious = "";
  var cursorPriority = "";
  function clearCursor() {
    if (!cursorTarget) return;
    cursorTarget.style.setProperty("cursor", cursorPrevious, cursorPriority);
    cursorTarget = null;
  }
  function showCursor(target, edge) {
    if (cursorTarget !== target) {
      clearCursor();
      cursorTarget = target;
      cursorPrevious = target.style.getPropertyValue("cursor");
      cursorPriority = target.style.getPropertyPriority("cursor");
    }
    target.style.setProperty("cursor", CURSORS[edge], "important");
  }

  document.addEventListener(
    "mousemove",
    function (e) {
      var edge = e.buttons === 0 ? edgeAt(e.clientX, e.clientY) : "";
      if (!edge) {
        clearCursor();
        return;
      }
      refreshFrameState();
      if (edgeResizable && e.target instanceof HTMLElement) showCursor(e.target, edge);
      else clearCursor();
    },
    true,
  );

  document.addEventListener(
    "mousedown",
    function (e) {
      if (e.button !== 0 || !(e.target instanceof Element)) return;
      var target = e.target;

      var handle = target.closest("[data-tynd-resize]");
      var handleDirection = handle && DIRECTIONS[handle.getAttribute("data-tynd-resize")];
      if (handleDirection) {
        e.preventDefault();
        call("startResizeDragging", { direction: handleDirection });
        return;
      }

      var edge = edgeAt(e.clientX, e.clientY);
      if (edge && edgeResizable) {
        e.preventDefault();
        call("startResizeDragging", { direction: DIRECTIONS[edge] });
        return;
      }

      var region = target.closest("[data-tynd-drag],[data-tynd-no-drag]");
      if (!region || !region.hasAttribute("data-tynd-drag")) return;
      var control = target.closest(CONTROLS);
      if (control && region.contains(control)) return;
      e.preventDefault();
      if (e.detail === 2) call("toggleMaximize");
      else call("startDragging");
    },
    true,
  );

  document.addEventListener(
    "dragstart",
    function (e) {
      if (!(e.target instanceof Element)) return;
      var source = e.target.closest("[data-tynd-drag-file]");
      if (!source) return;
      var files = (source.getAttribute("data-tynd-drag-file") || "")
        .split(/\r?\n/)
        .map(function (path) {
          return path.trim();
        })
        .filter(Boolean);
      if (!files.length) return;
      e.preventDefault();
      call("startDrag", { files: files });
    },
    true,
  );
})();
