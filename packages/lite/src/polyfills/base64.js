// atob / btoa — browser-standard base64 helpers.
// Accepts/produces only Latin-1 strings, per WHATWG spec.
(function () {
  var A = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  var LOOKUP = new Uint8Array(128);
  for (var k = 0; k < 128; k++) LOOKUP[k] = 255;
  for (var j = 0; j < A.length; j++) LOOKUP[A.charCodeAt(j)] = j;

  globalThis.btoa = function (str) {
    str = String(str);
    var out = "";
    var i = 0;
    while (i < str.length) {
      var c1 = str.charCodeAt(i++);
      var c2 = i < str.length ? str.charCodeAt(i++) : -1;
      var c3 = i < str.length ? str.charCodeAt(i++) : -1;
      if (c1 > 0xff || (c2 !== -1 && c2 > 0xff) || (c3 !== -1 && c3 > 0xff)) {
        throw new Error("btoa: string contains characters outside Latin-1");
      }
      var b1 = c1 >> 2;
      var b2 = ((c1 & 0x3) << 4) | (c2 === -1 ? 0 : c2 >> 4);
      var b3 = c2 === -1 ? 64 : ((c2 & 0xf) << 2) | (c3 === -1 ? 0 : c3 >> 6);
      var b4 = c3 === -1 ? 64 : c3 & 0x3f;
      out += A.charAt(b1) + A.charAt(b2)
        + (b3 === 64 ? "=" : A.charAt(b3))
        + (b4 === 64 ? "=" : A.charAt(b4));
    }
    return out;
  };

  globalThis.atob = function (str) {
    str = String(str).replace(/=+$/, "");
    if (str.length % 4 === 1) throw new Error("atob: invalid base64 length");
    var out = "";
    var bits = 0;
    var acc = 0;
    for (var i = 0; i < str.length; i++) {
      var code = str.charCodeAt(i);
      var v = code < 128 ? LOOKUP[code] : 255;
      if (v === 255) throw new Error("atob: invalid character");
      acc = (acc << 6) | v;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        out += String.fromCharCode((acc >> bits) & 0xff);
      }
    }
    return out;
  };
})();
