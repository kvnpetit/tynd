(function () {
  var id = "__tynd_error_overlay__";
  var old = document.getElementById(id);
  if (old) old.remove();

  // biome-ignore lint/correctness/noUndeclaredVariables: placeholder replaced with the JSON payload by eval_show_error_overlay (lib.rs)
  var payload = __TYND_OVERLAY_PAYLOAD__;
  var prevActive = document.activeElement;

  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).catch(function () {
        // Clipboard permission denied: nothing else to try.
      });
      return;
    }
    var ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "true");
    ta.style.cssText = "position:fixed;left:-9999px;top:-9999px;";
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand("copy");
    } catch (_) {
      // Legacy copy fallback unavailable: the text stays selectable in the overlay.
    }
    ta.remove();
  }

  function button(label, text, btnId) {
    var b = document.createElement("button");
    b.id = btnId;
    b.type = "button";
    b.textContent = label;
    b.style.cssText =
      "border:1px solid #8f9ba8;background:#20242b;color:#eef2f8;" +
      "border-radius:6px;padding:8px 10px;font:inherit;cursor:pointer;";
    b.addEventListener("click", function () {
      copyText(text);
      b.textContent = "Copied";
      window.setTimeout(function () {
        b.textContent = label;
      }, 1200);
    });
    return b;
  }

  var d = document.createElement("div");
  d.id = id;
  d.setAttribute("role", "alertdialog");
  d.setAttribute("aria-modal", "true");
  d.setAttribute("aria-labelledby", "__tynd_error_overlay_title__");
  d.tabIndex = -1;
  d.style.cssText =
    "position:fixed;inset:0;z-index:2147483647;background:#101217;" +
    "color:#eef2f8;font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;" +
    "padding:32px;overflow:auto;outline:none;";

  var wrap = document.createElement("div");
  wrap.style.cssText = "max-width:960px;margin:0 auto;";

  var code = document.createElement("div");
  code.textContent = payload.code || "BACKEND_ERROR";
  code.style.cssText = "color:#9ddcff;margin:0 0 8px;font-size:13px;font-weight:700;";

  var h = document.createElement("h2");
  h.id = "__tynd_error_overlay_title__";
  h.textContent = payload.title || "Backend error";
  h.style.cssText = "margin:0 0 10px;font-size:22px;line-height:1.2;font-weight:700;";

  var msg = document.createElement("div");
  msg.textContent = payload.message || "The backend failed during startup or reload.";
  msg.style.cssText = "margin:0 0 18px;color:#ffd6d6;font-size:15px;";

  var actions = document.createElement("div");
  actions.style.cssText = "display:flex;flex-wrap:wrap;gap:8px;margin:0 0 18px;";
  actions.appendChild(
    button("Copy report command", payload.reportCommand || "tynd report", "__tynd_copy_report__"),
  );
  actions.appendChild(
    button(
      "Copy fix command",
      payload.fixCommand || "tynd doctor --fix --dry-run",
      "__tynd_copy_fix__",
    ),
  );
  actions.appendChild(
    button("Copy error", payload.stack || payload.message || "", "__tynd_copy_error__"),
  );

  var hints = document.createElement("pre");
  hints.style.cssText =
    "margin:0 0 18px;padding:12px;border:1px solid #303844;border-radius:6px;" +
    "background:#171b22;color:#d8e0eb;white-space:pre-wrap;font:inherit;";
  hints.textContent =
    "Run: " +
    (payload.fixCommand || "tynd doctor --fix --dry-run") +
    "\nRun: " +
    (payload.reportCommand || "tynd report") +
    "\nExplain: " +
    (payload.explainCommand || "tynd explain BACKEND_ERROR");

  var pre = document.createElement("pre");
  pre.style.cssText =
    "margin:0;padding:16px;border-radius:6px;background:#05070a;color:#f7c7c7;" +
    "white-space:pre-wrap;overflow:auto;font:inherit;";
  pre.textContent = payload.stack || payload.message || "";

  wrap.appendChild(code);
  wrap.appendChild(h);
  wrap.appendChild(msg);
  wrap.appendChild(actions);
  wrap.appendChild(hints);
  wrap.appendChild(pre);
  d.appendChild(wrap);

  d.addEventListener("keydown", function (e) {
    if (e.key === "Tab") {
      e.preventDefault();
      d.focus();
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
      copyText(payload.stack || payload.message || "");
    }
  });

  if (document.body) document.body.appendChild(d);
  else document.documentElement.appendChild(d);
  d.focus();

  var mo = new MutationObserver(function () {
    if (!document.getElementById(id)) {
      mo.disconnect();
      if (prevActive && typeof prevActive.focus === "function") prevActive.focus();
    }
  });
  mo.observe(document.documentElement, { childList: true, subtree: true });
})();
