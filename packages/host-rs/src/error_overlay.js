(function () {
  var id = "__tynd_error_overlay__";
  var old = document.getElementById(id);
  if (old) old.remove();
  var d = document.createElement("div");
  d.id = id;
  d.style.cssText =
    "position:fixed;inset:0;z-index:2147483647;background:rgba(20,20,20,0.92);" +
    "color:#fce5e5;font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;" +
    "padding:32px;overflow:auto;white-space:pre-wrap;";
  d.textContent = "⚠  Backend error\n\n" + __TYND_OVERLAY_MSG__;
  if (document.body) document.body.appendChild(d);
  else document.documentElement.appendChild(d);
})();
