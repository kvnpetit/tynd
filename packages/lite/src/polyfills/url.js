// URL / URLSearchParams — pragmatic subset of the WHATWG URL spec.
// Covers the shapes web code actually uses: parsing, mutation of
// protocol/host/port/pathname/search/hash, SearchParams iteration.
// Not a full RFC-3986 parser — no IDN, no punycode, no IPv6 literal quirks.
(function () {
  function percentEncode(str, safe) {
    // Encode every byte of the UTF-8 serialisation of `str` that isn't in `safe`.
    var enc = new TextEncoder().encode(str);
    var out = "";
    for (var i = 0; i < enc.length; i++) {
      var b = enc[i];
      var c = String.fromCharCode(b);
      if (
        (b >= 0x30 && b <= 0x39) || // 0-9
        (b >= 0x41 && b <= 0x5a) || // A-Z
        (b >= 0x61 && b <= 0x7a) || // a-z
        safe.indexOf(c) >= 0
      ) {
        out += c;
      } else {
        out += "%" + (b < 16 ? "0" : "") + b.toString(16).toUpperCase();
      }
    }
    return out;
  }
  function percentDecode(str) {
    var bytes = [];
    for (var i = 0; i < str.length; i++) {
      var ch = str.charAt(i);
      if (ch === "%" && i + 2 < str.length) {
        var v = parseInt(str.substr(i + 1, 2), 16);
        if (!Number.isNaN(v)) { bytes.push(v); i += 2; continue; }
      }
      if (ch === "+") { bytes.push(0x20); continue; }
      bytes.push(ch.charCodeAt(0) & 0xff);
    }
    return new TextDecoder().decode(new Uint8Array(bytes));
  }
  var QUERY_SAFE = "-._~!$&'()*+,;=:@/?";
  var PATH_SAFE = "-._~!$&'()*+,;=:@/";

  function URLSearchParams(init) {
    this._list = [];
    if (init == null) return;
    if (typeof init === "string") {
      if (init.charAt(0) === "?") init = init.slice(1);
      if (init) {
        var pairs = init.split("&");
        for (var i = 0; i < pairs.length; i++) {
          var eq = pairs[i].indexOf("=");
          var k = eq < 0 ? pairs[i] : pairs[i].slice(0, eq);
          var v = eq < 0 ? "" : pairs[i].slice(eq + 1);
          this._list.push([percentDecode(k), percentDecode(v)]);
        }
      }
    } else if (init instanceof URLSearchParams) {
      for (var j = 0; j < init._list.length; j++) this._list.push([init._list[j][0], init._list[j][1]]);
    } else if (Array.isArray(init)) {
      for (var k2 = 0; k2 < init.length; k2++) this._list.push([String(init[k2][0]), String(init[k2][1])]);
    } else if (typeof init === "object") {
      for (var key in init) this._list.push([key, String(init[key])]);
    }
  }
  URLSearchParams.prototype.append = function (k, v) { this._list.push([String(k), String(v)]); };
  URLSearchParams.prototype.delete = function (k) {
    k = String(k);
    this._list = this._list.filter(function (p) { return p[0] !== k; });
  };
  URLSearchParams.prototype.get = function (k) {
    k = String(k);
    for (var i = 0; i < this._list.length; i++) if (this._list[i][0] === k) return this._list[i][1];
    return null;
  };
  URLSearchParams.prototype.getAll = function (k) {
    k = String(k);
    return this._list.filter(function (p) { return p[0] === k; }).map(function (p) { return p[1]; });
  };
  URLSearchParams.prototype.has = function (k) {
    k = String(k);
    for (var i = 0; i < this._list.length; i++) if (this._list[i][0] === k) return true;
    return false;
  };
  URLSearchParams.prototype.set = function (k, v) {
    k = String(k); v = String(v);
    var found = false;
    this._list = this._list.filter(function (p) {
      if (p[0] !== k) return true;
      if (!found) { p[1] = v; found = true; return true; }
      return false;
    });
    if (!found) this._list.push([k, v]);
  };
  URLSearchParams.prototype.sort = function () {
    this._list.sort(function (a, b) { return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0; });
  };
  URLSearchParams.prototype.toString = function () {
    return this._list
      .map(function (p) {
        // Form-urlencoded: percent-encode then swap %20 (space) for '+'.
        return percentEncode(p[0], "").replace(/%20/g, "+") + "=" + percentEncode(p[1], "").replace(/%20/g, "+");
      })
      .join("&");
  };
  URLSearchParams.prototype.forEach = function (cb, self) {
    for (var i = 0; i < this._list.length; i++) cb.call(self, this._list[i][1], this._list[i][0], this);
  };
  URLSearchParams.prototype.entries = function* () {
    for (var i = 0; i < this._list.length; i++) yield [this._list[i][0], this._list[i][1]];
  };
  URLSearchParams.prototype.keys = function* () {
    for (var i = 0; i < this._list.length; i++) yield this._list[i][0];
  };
  URLSearchParams.prototype.values = function* () {
    for (var i = 0; i < this._list.length; i++) yield this._list[i][1];
  };
  URLSearchParams.prototype[Symbol.iterator] = URLSearchParams.prototype.entries;

  var URL_RE = /^([a-z][a-z0-9+\-.]*):\/\/(?:([^/?#@]*)@)?([^/?#:]+)(?::(\d+))?([^?#]*)(\?[^#]*)?(#.*)?$/i;
  var OPAQUE_RE = /^([a-z][a-z0-9+\-.]*):([^?#]*)(\?[^#]*)?(#.*)?$/i;

  function URL(input, base) {
    var src = String(input);
    if (base != null && !URL_RE.test(src) && !OPAQUE_RE.test(src)) {
      // Resolve against base (very minimal: absolute paths, same-origin).
      var baseUrl = new URL(base);
      if (src.charAt(0) === "#") src = baseUrl.origin + baseUrl.pathname + baseUrl.search + src;
      else if (src.charAt(0) === "?") src = baseUrl.origin + baseUrl.pathname + src;
      else if (src.charAt(0) === "/") src = baseUrl.origin + src;
      else {
        var path = baseUrl.pathname.replace(/\/[^/]*$/, "/");
        src = baseUrl.origin + path + src;
      }
    }
    var m = URL_RE.exec(src);
    if (m) {
      this.protocol = m[1].toLowerCase() + ":";
      this.username = m[2] ? m[2].split(":")[0] || "" : "";
      this.password = m[2] && m[2].indexOf(":") >= 0 ? m[2].split(":").slice(1).join(":") : "";
      this.hostname = m[3].toLowerCase();
      this.port = m[4] || "";
      this.pathname = m[5] || "/";
      this.search = m[6] || "";
      this.hash = m[7] || "";
    } else {
      var o = OPAQUE_RE.exec(src);
      if (!o) throw new TypeError("Invalid URL: " + input);
      this.protocol = o[1].toLowerCase() + ":";
      this.username = "";
      this.password = "";
      this.hostname = "";
      this.port = "";
      this.pathname = o[2] || "";
      this.search = o[3] || "";
      this.hash = o[4] || "";
    }
    this._searchParams = new URLSearchParams(this.search);
    var self = this;
    // Keep search/searchParams in sync when searchParams mutates.
    var _toString = this._searchParams.toString.bind(this._searchParams);
    this._searchParams.toString = function () {
      var s = _toString();
      self.search = s ? "?" + s : "";
      return s;
    };
  }
  Object.defineProperty(URL.prototype, "host", {
    get: function () { return this.port ? this.hostname + ":" + this.port : this.hostname; },
    set: function (v) {
      var i = v.indexOf(":");
      if (i >= 0) { this.hostname = v.slice(0, i); this.port = v.slice(i + 1); }
      else { this.hostname = v; this.port = ""; }
    },
  });
  Object.defineProperty(URL.prototype, "origin", {
    get: function () {
      if (!this.hostname) return "null";
      return this.protocol + "//" + this.hostname + (this.port ? ":" + this.port : "");
    },
  });
  Object.defineProperty(URL.prototype, "href", {
    get: function () { return this.toString(); },
    set: function (v) {
      var u = new URL(v);
      this.protocol = u.protocol; this.hostname = u.hostname; this.port = u.port;
      this.username = u.username; this.password = u.password;
      this.pathname = u.pathname; this.search = u.search; this.hash = u.hash;
      this._searchParams = new URLSearchParams(this.search);
    },
  });
  Object.defineProperty(URL.prototype, "searchParams", {
    get: function () { return this._searchParams; },
  });
  URL.prototype.toString = function () {
    var auth = this.username
      ? this.username + (this.password ? ":" + this.password : "") + "@"
      : "";
    var host = this.hostname ? "//" + auth + this.host : "";
    return this.protocol + host + this.pathname + this.search + this.hash;
  };
  URL.prototype.toJSON = URL.prototype.toString;

  globalThis.URL = URL;
  globalThis.URLSearchParams = URLSearchParams;
})();
