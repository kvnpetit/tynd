// Debug helper injected into every webview when TYND_DEBUG_SOCKET /
// TYND_DEBUG_PORT is set. Exposes __tynd_debug_*__ globals consumed by
// the host's `webview.*` debug methods. Buffered console + network
// logs let an attached MCP agent inspect history without re-running
// the user flow.
(function () {
  "use strict";
  if (window.__tynd_debug_helper_loaded__) return;
  window.__tynd_debug_helper_loaded__ = true;

  var REF_ATTR = "data-tynd-debug-ref";
  var refMap = new Map();
  var nextRef = 1;

  function assignRef(el) {
    if (!el) return null;
    var existing = el.getAttribute(REF_ATTR);
    if (existing) return existing;
    var ref = "r" + nextRef++;
    el.setAttribute(REF_ATTR, ref);
    refMap.set(ref, el);
    return ref;
  }

  function elFromRef(ref) {
    var cached = refMap.get(ref);
    if (cached && document.contains(cached)) return cached;
    var found = document.querySelector("[" + REF_ATTR + "='" + ref + "']");
    if (found) refMap.set(ref, found);
    return found;
  }

  function bbox(el) {
    var r = el.getBoundingClientRect();
    return {
      x: Math.round(r.left),
      y: Math.round(r.top),
      w: Math.round(r.width),
      h: Math.round(r.height),
    };
  }

  function visibleText(el) {
    // biome-ignore lint/performance/useDomNodeTextContent: the snapshot reports the rendered (visible) text, which only innerText gives
    var rendered = el.innerText;
    var t = (
      rendered ||
      el.textContent ||
      el.value ||
      el.placeholder ||
      (el.getAttribute && el.getAttribute("aria-label")) ||
      ""
    ).trim();
    return t.length > 200 ? t.slice(0, 200) + "…" : t;
  }

  window.__tynd_debug_snapshot__ = function (mode) {
    mode = mode || "map";
    if (mode === "html") {
      return { html: document.documentElement.outerHTML };
    }
    if (mode === "state") {
      return {
        url: location.href,
        title: document.title,
        scrollX: window.scrollX,
        scrollY: window.scrollY,
        viewport: { w: window.innerWidth, h: window.innerHeight },
        readyState: document.readyState,
      };
    }
    var nodes = [];
    var all = document.querySelectorAll(
      "button, a, input, textarea, select, [role=button], [role=link]," +
        "[role=menuitem], [role=tab], [contenteditable=true]",
    );
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      var rect = el.getBoundingClientRect();
      if (rect.width === 0 || rect.height === 0) continue;
      var ref = assignRef(el);
      var box = bbox(el);
      nodes.push({
        ref: ref,
        tag: el.tagName.toLowerCase(),
        text: visibleText(el),
        x: box.x,
        y: box.y,
        w: box.w,
        h: box.h,
      });
    }
    return { map: nodes, viewport: { w: window.innerWidth, h: window.innerHeight } };
  };

  window.__tynd_debug_find__ = function (selector) {
    try {
      var el = document.querySelector(selector);
      if (!el) return null;
      var ref = assignRef(el);
      var box = bbox(el);
      return { ref: ref, x: box.x, y: box.y, w: box.w, h: box.h };
    } catch (e) {
      return { error: String(e) };
    }
  };

  function dispatchClick(el, x, y) {
    var opts = {
      bubbles: true,
      cancelable: true,
      view: window,
      clientX: x,
      clientY: y,
      button: 0,
    };
    el.dispatchEvent(new MouseEvent("mousedown", opts));
    el.dispatchEvent(new MouseEvent("mouseup", opts));
    el.dispatchEvent(new MouseEvent("click", opts));
  }

  function resolveTarget(target) {
    var el, x, y;
    if (typeof target === "object" && target !== null) {
      if (target.ref) {
        el = elFromRef(target.ref);
        if (!el) return { error: "ref not found" };
        var b = bbox(el);
        x = b.x + Math.floor(b.w / 2);
        y = b.y + Math.floor(b.h / 2);
      } else if (typeof target.x === "number" && typeof target.y === "number") {
        x = target.x;
        y = target.y;
        el = document.elementFromPoint(x, y);
        if (!el) return { error: "no element at point" };
      } else {
        return { error: "ref or {x,y} required" };
      }
    } else {
      return { error: "object target required" };
    }
    return { el: el, x: x, y: y };
  }

  window.__tynd_debug_click__ = function (target) {
    var resolved = resolveTarget(target);
    if (resolved.error) return { ok: false, error: resolved.error };
    var el = resolved.el;
    var x = resolved.x;
    var y = resolved.y;
    if (typeof el.focus === "function") {
      try {
        el.focus();
      } catch (_) {
        // Some elements reject focus; the click below is still dispatched.
      }
    }
    dispatchClick(el, x, y);
    return { ok: true, ref: el.getAttribute(REF_ATTR), x: x, y: y };
  };

  function setNativeValue(el, value) {
    var proto = Object.getPrototypeOf(el);
    var setter = Object.getOwnPropertyDescriptor(proto, "value");
    if (setter && setter.set) {
      setter.set.call(el, value);
    } else {
      el.value = value;
    }
  }

  window.__tynd_debug_type__ = function (text, target) {
    var el;
    if (target && target.ref) {
      el = elFromRef(target.ref);
    } else {
      el = document.activeElement;
    }
    if (!el) return { ok: false, error: "no target element" };
    if (el.isContentEditable) {
      el.focus();
      // Lexical / Slate / ProseMirror listen on InputEvent.
      var ev = new InputEvent("beforeinput", {
        bubbles: true,
        cancelable: true,
        data: text,
        inputType: "insertText",
      });
      el.dispatchEvent(ev);
      // Fallback: insert raw text node so plain contenteditable updates.
      try {
        document.execCommand("insertText", false, text);
      } catch (_) {
        // execCommand is deprecated and may throw; the input event below still fires.
      }
      el.dispatchEvent(
        new InputEvent("input", { bubbles: true, data: text, inputType: "insertText" }),
      );
      return { ok: true, ref: el.getAttribute(REF_ATTR) };
    }
    if (el.tagName === "INPUT" || el.tagName === "TEXTAREA") {
      el.focus();
      setNativeValue(el, (el.value || "") + text);
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return { ok: true, ref: el.getAttribute(REF_ATTR) };
    }
    return { ok: false, error: "target not editable" };
  };

  function replaceFieldValue(el, value) {
    if (el.isContentEditable) {
      el.focus();
      el.textContent = value;
      el.dispatchEvent(
        new InputEvent("input", {
          bubbles: true,
          data: value,
          inputType: "insertText",
        }),
      );
      return true;
    }
    if (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT") {
      el.focus();
      setNativeValue(el, value);
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
      return true;
    }
    return false;
  }

  window.__tynd_debug_fill__ = function (fields) {
    if (!Array.isArray(fields) || fields.length === 0) {
      return { ok: false, error: "at least one field required" };
    }
    var filled = [];
    for (var i = 0; i < fields.length; i++) {
      var field = fields[i] || {};
      var el = elFromRef(field.ref);
      if (!el) return { ok: false, error: "ref not found: " + field.ref, filled: filled };
      if (!replaceFieldValue(el, String(field.value))) {
        return { ok: false, error: "target not editable: " + field.ref, filled: filled };
      }
      filled.push(field.ref);
    }
    return { ok: true, filled: filled };
  };

  window.__tynd_debug_press_key__ = function (combo) {
    if (typeof combo !== "string" || !combo.trim()) {
      return { ok: false, error: "key required" };
    }
    var parts = combo.split("+");
    var key = parts.pop();
    var modifiers = parts.map(function (part) {
      return part.toLowerCase();
    });
    var isMac = /Mac|iPhone|iPad/.test(navigator.platform || "");
    var options = {
      key: key,
      code: key && key.length === 1 ? "Key" + key.toUpperCase() : key,
      bubbles: true,
      cancelable: true,
      ctrlKey: modifiers.indexOf("ctrl") >= 0 || (!isMac && modifiers.indexOf("mod") >= 0),
      metaKey: modifiers.indexOf("meta") >= 0 || (isMac && modifiers.indexOf("mod") >= 0),
      altKey: modifiers.indexOf("alt") >= 0,
      shiftKey: modifiers.indexOf("shift") >= 0,
    };
    var el = document.activeElement || document.body;
    var accepted = el.dispatchEvent(new KeyboardEvent("keydown", options));
    el.dispatchEvent(new KeyboardEvent("keyup", options));
    return { ok: true, key: key, accepted: accepted };
  };

  window.__tynd_debug_scroll__ = function (target, opts) {
    opts = opts || {};
    if (target === "top") {
      window.scrollTo({ top: 0, behavior: opts.behavior || "auto" });
      return { ok: true };
    }
    if (target === "bottom") {
      window.scrollTo({ top: document.body.scrollHeight, behavior: opts.behavior || "auto" });
      return { ok: true };
    }
    if (typeof target === "object" && target !== null) {
      if (target.ref) {
        var el = elFromRef(target.ref);
        if (!el) return { ok: false, error: "ref not found" };
        el.scrollIntoView({ behavior: opts.behavior || "auto", block: opts.block || "center" });
        return { ok: true };
      }
      if (typeof target.dy === "number" || typeof target.dx === "number") {
        window.scrollBy({
          left: target.dx || 0,
          top: target.dy || 0,
          behavior: opts.behavior || "auto",
        });
        return { ok: true };
      }
    }
    return { ok: false, error: "unknown scroll target" };
  };

  window.__tynd_debug_hover__ = function (target) {
    var resolved = resolveTarget(target);
    if (resolved.error) return { ok: false, error: resolved.error };
    var opts = {
      bubbles: true,
      cancelable: true,
      view: window,
      clientX: resolved.x,
      clientY: resolved.y,
    };
    resolved.el.dispatchEvent(new MouseEvent("mouseover", opts));
    resolved.el.dispatchEvent(new MouseEvent("mouseenter", opts));
    resolved.el.dispatchEvent(new MouseEvent("mousemove", opts));
    return { ok: true, ref: resolved.el.getAttribute(REF_ATTR) };
  };

  window.__tynd_debug_drag__ = function (start, end) {
    var from = resolveTarget(start);
    var to = resolveTarget(end);
    if (from.error) return { ok: false, error: "start: " + from.error };
    if (to.error) return { ok: false, error: "end: " + to.error };
    var down = {
      bubbles: true,
      cancelable: true,
      view: window,
      button: 0,
      clientX: from.x,
      clientY: from.y,
    };
    var move = {
      bubbles: true,
      cancelable: true,
      view: window,
      button: 0,
      clientX: to.x,
      clientY: to.y,
    };
    from.el.dispatchEvent(new MouseEvent("mousedown", down));
    from.el.dispatchEvent(new MouseEvent("dragstart", down));
    to.el.dispatchEvent(new MouseEvent("mousemove", move));
    to.el.dispatchEvent(new MouseEvent("dragover", move));
    to.el.dispatchEvent(new MouseEvent("drop", move));
    to.el.dispatchEvent(new MouseEvent("mouseup", move));
    from.el.dispatchEvent(new MouseEvent("dragend", move));
    return { ok: true };
  };

  // === Console buffer ===
  var consoleBuf = [];
  var CONSOLE_MAX = 1000;
  ["log", "warn", "error", "debug", "info"].forEach(function (level) {
    var orig = console[level];
    console[level] = function () {
      try {
        var args = Array.prototype.slice.call(arguments).map(function (a) {
          if (typeof a === "string") return a;
          try {
            return JSON.stringify(a);
          } catch (_) {
            return String(a);
          }
        });
        if (consoleBuf.length >= CONSOLE_MAX) consoleBuf.shift();
        consoleBuf.push({ ts: Date.now(), level: level, args: args });
      } catch (_) {
        // Buffering must never break the page's own console call.
      }
      return orig.apply(console, arguments);
    };
  });
  window.__tynd_debug_console__ = function (clear) {
    var snap = consoleBuf.slice();
    if (clear) consoleBuf.length = 0;
    return snap;
  };

  // === Network buffer ===
  var netBuf = [];
  var NET_MAX = 500;
  function pushNet(entry) {
    if (netBuf.length >= NET_MAX) netBuf.shift();
    netBuf.push(entry);
  }

  if (typeof window.fetch === "function") {
    var origFetch = window.fetch.bind(window);
    window.fetch = function (input, init) {
      var url = typeof input === "string" ? input : (input && input.url) || "";
      var method = (init && init.method) || (input && input.method) || "GET";
      var started = Date.now();
      var entry = {
        ts: started,
        kind: "fetch",
        method: method,
        url: url,
        status: null,
        durationMs: null,
        error: null,
      };
      pushNet(entry);
      return origFetch(input, init).then(
        function (r) {
          entry.status = r.status;
          entry.durationMs = Date.now() - started;
          return r;
        },
        function (e) {
          entry.error = String(e);
          entry.durationMs = Date.now() - started;
          throw e;
        },
      );
    };
  }

  if (typeof window.XMLHttpRequest === "function") {
    var XO = window.XMLHttpRequest.prototype.open;
    var XS = window.XMLHttpRequest.prototype.send;
    window.XMLHttpRequest.prototype.open = function (method, url) {
      this.__tynd_debug_meta = { method: method, url: url };
      return XO.apply(this, arguments);
    };
    window.XMLHttpRequest.prototype.send = function () {
      var meta = this.__tynd_debug_meta || {};
      var started = Date.now();
      var entry = {
        ts: started,
        kind: "xhr",
        method: meta.method,
        url: meta.url,
        status: null,
        durationMs: null,
        error: null,
      };
      pushNet(entry);
      var xhr = this;
      this.addEventListener("loadend", function () {
        entry.status = xhr.status;
        entry.durationMs = Date.now() - started;
      });
      this.addEventListener("error", function () {
        entry.error = "xhr error";
        entry.durationMs = Date.now() - started;
      });
      return XS.apply(this, arguments);
    };
  }

  window.__tynd_debug_network__ = function (clear) {
    var snap = netBuf.slice();
    if (clear) netBuf.length = 0;
    return snap;
  };
})();
