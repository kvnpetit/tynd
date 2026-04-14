// Injected into every page via webview_init(). Sets up window.__bunview_api__.

export const CLIENT_SCRIPT = /* js */`
(function () {
  var listeners = new Map();

  function _dispatch(name, payload) {
    var handlers = listeners.get(name);
    if (handlers) for (var cb of handlers) try { cb(payload); } catch(e) {}
  }

  function invoke(command, payload) {
    if (typeof window.__bunview__ !== "function") {
      return Promise.reject(new Error(
        "[bunview] Native IPC bridge (window.__bunview__) not available."
      ));
    }
    return window.__bunview__(JSON.stringify({ command: command, payload: payload }))
      .then(function (raw) {
        if (raw === null || raw === undefined) return raw;
        if (typeof raw === "string") {
          try { return JSON.parse(raw); } catch { return raw; }
        }
        return raw;
      });
  }

  function emit(event, payload) {
    if (typeof window.__bv_emit__ === "function") {
      window.__bv_emit__(JSON.stringify({ name: event, payload: payload }));
    }
  }

  function listen(event, handler) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(handler);
    return function () { listeners.get(event)?.delete(handler); };
  }

  function once(event, handler) {
    var unlisten = listen(event, function (payload) {
      unlisten();
      handler(payload);
    });
    return unlisten;
  }

  window.__bunview_api__ = {
    invoke:    invoke,
    listen:    listen,
    once:      once,
    emit:      emit,
    _dispatch: _dispatch,
    isConnected:      function () { return true; },
    onConnectionChange: function (cb) { cb(true); return function () {}; },
  };

})();
`;
