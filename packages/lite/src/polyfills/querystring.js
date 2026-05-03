// node:querystring — legacy URL query string codec.
// Spec parity with Node's `querystring`. URLSearchParams is the modern API
// but a lot of npm libs still reach for `qs.stringify` / `querystring.parse`.
(function () {
  function unescapeBuffer(s, decodeSpaces) {
    var out = "", i = 0, n = s.length;
    while (i < n) {
      var c = s.charAt(i);
      if (c === "+" && decodeSpaces) { out += " "; i++; }
      else if (c === "%" && i + 2 < n) {
        var hex = s.substr(i + 1, 2), v = parseInt(hex, 16);
        if (!isNaN(v)) { out += String.fromCharCode(v); i += 3; continue; }
        out += c; i++;
      } else { out += c; i++; }
    }
    return out;
  }
  function qsUnescape(s, decodeSpaces) {
    try {
      return decodeURIComponent(decodeSpaces ? s.replace(/\+/g, " ") : s);
    } catch (_) { return unescapeBuffer(s, decodeSpaces); }
  }
  function qsEscape(s) {
    if (typeof s !== "string") {
      if (typeof s === "object") s = String(s);
      else s = "" + s;
    }
    return encodeURIComponent(s).replace(/[!'()*]/g, function (c) {
      return "%" + c.charCodeAt(0).toString(16).toUpperCase();
    });
  }
  function stringify(obj, sep, eq, _opts) {
    sep = sep || "&"; eq = eq || "=";
    if (obj === null || typeof obj !== "object") return "";
    var keys = Object.keys(obj);
    var out = "";
    for (var i = 0; i < keys.length; i++) {
      var k = keys[i], v = obj[k], ks = qsEscape(k);
      if (Array.isArray(v)) {
        for (var j = 0; j < v.length; j++) {
          if (out.length) out += sep;
          out += ks + eq + qsEscape(v[j]);
        }
      } else {
        if (out.length) out += sep;
        out += ks + eq + qsEscape(v == null ? "" : v);
      }
    }
    return out;
  }
  function parse(qs, sep, eq, opts) {
    sep = sep || "&"; eq = eq || "=";
    var result = Object.create(null);
    if (typeof qs !== "string" || qs.length === 0) return result;
    var maxKeys = (opts && typeof opts.maxKeys === "number") ? opts.maxKeys : 1000;
    var pairs = qs.split(sep);
    var len = maxKeys > 0 ? Math.min(pairs.length, maxKeys) : pairs.length;
    for (var i = 0; i < len; i++) {
      var p = pairs[i];
      if (!p) continue;
      var idx = p.indexOf(eq);
      var k, v;
      if (idx >= 0) { k = p.slice(0, idx); v = p.slice(idx + eq.length); }
      else { k = p; v = ""; }
      k = qsUnescape(k, true);
      v = qsUnescape(v, true);
      if (Object.prototype.hasOwnProperty.call(result, k)) {
        if (Array.isArray(result[k])) result[k].push(v);
        else result[k] = [result[k], v];
      } else result[k] = v;
    }
    return result;
  }

  var querystring = {
    stringify: stringify, encode: stringify,
    parse: parse, decode: parse,
    escape: qsEscape, unescape: qsUnescape,
  };
  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["querystring"] = querystring;
  globalThis.__tynd_modules["node:querystring"] = querystring;
})();
