// TextEncoder / TextDecoder — UTF-8 only (WHATWG spec default).
(function () {
  function TextEncoder() {}
  TextEncoder.prototype.encoding = "utf-8";
  TextEncoder.prototype.encode = function (input) {
    var str = String(input == null ? "" : input);
    var bytes = [];
    for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) {
        var c2 = str.charCodeAt(i + 1);
        if (c2 >= 0xdc00 && c2 <= 0xdfff) {
          c = 0x10000 + ((c - 0xd800) << 10) + (c2 - 0xdc00);
          i++;
        }
      }
      if (c < 0x80) {
        bytes.push(c);
      } else if (c < 0x800) {
        bytes.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
      } else if (c < 0x10000) {
        bytes.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
      } else {
        bytes.push(
          0xf0 | (c >> 18),
          0x80 | ((c >> 12) & 0x3f),
          0x80 | ((c >> 6) & 0x3f),
          0x80 | (c & 0x3f),
        );
      }
    }
    return new Uint8Array(bytes);
  };

  function TextDecoder(label, opts) {
    this.encoding = (label || "utf-8").toLowerCase();
    if (this.encoding !== "utf-8" && this.encoding !== "utf8") {
      throw new RangeError("TextDecoder: only utf-8 supported");
    }
    this.fatal = !!(opts && opts.fatal);
    this.ignoreBOM = !!(opts && opts.ignoreBOM);
    this._buf = null;
  }
  TextDecoder.prototype.decode = function (input, opts) {
    var stream = !!(opts && opts.stream);
    var bytes;
    if (input == null) bytes = new Uint8Array(0);
    else if (input instanceof Uint8Array) bytes = input;
    else if (input.buffer instanceof ArrayBuffer)
      bytes = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
    else if (input instanceof ArrayBuffer) bytes = new Uint8Array(input);
    else throw new TypeError("TextDecoder.decode: expected BufferSource");

    if (this._buf && this._buf.length) {
      var merged = new Uint8Array(this._buf.length + bytes.length);
      merged.set(this._buf, 0);
      merged.set(bytes, this._buf.length);
      bytes = merged;
      this._buf = null;
    }

    var out = "";
    var i = 0;
    var n = bytes.length;
    while (i < n) {
      var b = bytes[i];
      var need, cp;
      if (b < 0x80) { out += String.fromCharCode(b); i++; continue; }
      else if ((b & 0xe0) === 0xc0) { need = 1; cp = b & 0x1f; }
      else if ((b & 0xf0) === 0xe0) { need = 2; cp = b & 0x0f; }
      else if ((b & 0xf8) === 0xf0) { need = 3; cp = b & 0x07; }
      else {
        if (this.fatal) throw new TypeError("TextDecoder: invalid UTF-8");
        out += "\uFFFD"; i++; continue;
      }
      if (i + need >= n) {
        if (stream) {
          this._buf = bytes.slice(i);
          return out;
        }
        if (this.fatal) throw new TypeError("TextDecoder: truncated UTF-8");
        out += "\uFFFD"; i = n; break;
      }
      for (var k = 1; k <= need; k++) {
        var bb = bytes[i + k];
        if ((bb & 0xc0) !== 0x80) {
          if (this.fatal) throw new TypeError("TextDecoder: invalid UTF-8");
          cp = -1; break;
        }
        cp = (cp << 6) | (bb & 0x3f);
      }
      i += need + 1;
      if (cp < 0) { out += "\uFFFD"; continue; }
      if (cp > 0xffff) {
        cp -= 0x10000;
        out += String.fromCharCode(0xd800 + (cp >> 10), 0xdc00 + (cp & 0x3ff));
      } else {
        out += String.fromCharCode(cp);
      }
    }
    return out;
  };

  globalThis.TextEncoder = TextEncoder;
  globalThis.TextDecoder = TextDecoder;
})();
