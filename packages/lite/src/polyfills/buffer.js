// node:buffer / globalThis.Buffer — Node compat layer over Uint8Array.
//
// Implements the Buffer subset that 90% of npm uses:
//   Buffer.alloc / allocUnsafe / from(string/array/buffer/arrayBuffer) / concat
//   Buffer.byteLength / Buffer.isBuffer / Buffer.isEncoding / Buffer.compare
//   instance: write / toString / slice / subarray / copy / fill / equals /
//             compare / indexOf / includes / lastIndexOf / readUInt*BE/LE /
//             writeUInt*BE/LE / readInt*BE/LE / writeInt*BE/LE / readFloat /
//             writeFloat / readDouble / writeDouble / readBigInt64 etc.
// Encodings: utf8/utf-8 (default), ascii, latin1/binary, hex, base64,
// base64url, ucs2/utf16le.
//
// Implemented as a function with prototype = Uint8Array.prototype so
// `instanceof Buffer` and `instanceof Uint8Array` both work.
(function () {
  var TE = globalThis.TextEncoder ? new TextEncoder() : null;
  var TD = globalThis.TextDecoder ? new TextDecoder("utf-8") : null;
  var b64Encode = globalThis.__tynd_b64_encode;
  var b64Decode = globalThis.__tynd_b64_decode;

  function normalizeEncoding(enc) {
    if (enc === undefined || enc === null) return "utf8";
    var s = String(enc).toLowerCase();
    switch (s) {
      case "utf8": case "utf-8": return "utf8";
      case "ascii": return "ascii";
      case "latin1": case "binary": case "iso-8859-1": return "latin1";
      case "hex": return "hex";
      case "base64": return "base64";
      case "base64url": return "base64url";
      case "ucs2": case "ucs-2": case "utf16le": case "utf-16le": return "utf16le";
      default: return null;
    }
  }
  function isEncoding(enc) { return normalizeEncoding(enc) != null; }

  function utf8ByteLength(str) {
    if (TE) return TE.encode(str).length;
    var n = 0;
    for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      if (c < 0x80) n++;
      else if (c < 0x800) n += 2;
      else if (c >= 0xd800 && c <= 0xdbff) { n += 4; i++; }
      else n += 3;
    }
    return n;
  }
  function utf8Encode(str) {
    if (TE) return TE.encode(str);
    var out = []; for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      if (c < 0x80) out.push(c);
      else if (c < 0x800) out.push(0xc0 | (c >> 6), 0x80 | (c & 0x3f));
      else if (c >= 0xd800 && c <= 0xdbff && i + 1 < str.length) {
        var c2 = str.charCodeAt(++i);
        var cp = 0x10000 + ((c - 0xd800) << 10) + (c2 - 0xdc00);
        out.push(0xf0 | (cp >> 18), 0x80 | ((cp >> 12) & 0x3f), 0x80 | ((cp >> 6) & 0x3f), 0x80 | (cp & 0x3f));
      } else out.push(0xe0 | (c >> 12), 0x80 | ((c >> 6) & 0x3f), 0x80 | (c & 0x3f));
    }
    return new Uint8Array(out);
  }
  function utf8Decode(bytes, start, end) {
    if (start == null) start = 0;
    if (end == null) end = bytes.length;
    var sub = bytes.subarray(start, end);
    if (TD) return TD.decode(sub);
    var s = "";
    for (var i = 0; i < sub.length; i++) s += String.fromCharCode(sub[i]);
    try { return decodeURIComponent(escape(s)); } catch (_) { return s; }
  }
  function hexDecode(str) {
    if (str.length % 2 !== 0) str = str.slice(0, str.length - 1);
    var out = new Uint8Array(str.length / 2);
    for (var i = 0; i < out.length; i++) {
      var v = parseInt(str.substr(i * 2, 2), 16);
      if (Number.isNaN(v)) { return out.subarray(0, i); }
      out[i] = v;
    }
    return out;
  }
  function hexEncode(bytes) {
    var s = "";
    for (var i = 0; i < bytes.length; i++) {
      var h = bytes[i].toString(16);
      s += h.length === 1 ? "0" + h : h;
    }
    return s;
  }
  function base64UrlToStd(s) {
    s = s.replace(/-/g, "+").replace(/_/g, "/");
    while (s.length % 4) s += "=";
    return s;
  }
  function stdToBase64Url(s) {
    return s.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }
  function utf16leDecode(bytes) {
    var s = ""; for (var i = 0; i + 1 < bytes.length; i += 2) s += String.fromCharCode(bytes[i] | (bytes[i + 1] << 8));
    return s;
  }
  function utf16leEncode(str) {
    var out = new Uint8Array(str.length * 2);
    for (var i = 0; i < str.length; i++) {
      var c = str.charCodeAt(i);
      out[i * 2] = c & 0xff; out[i * 2 + 1] = (c >> 8) & 0xff;
    }
    return out;
  }
  function asciiEncode(str) {
    var out = new Uint8Array(str.length);
    for (var i = 0; i < str.length; i++) out[i] = str.charCodeAt(i) & 0x7f;
    return out;
  }
  function latin1Encode(str) {
    var out = new Uint8Array(str.length);
    for (var i = 0; i < str.length; i++) out[i] = str.charCodeAt(i) & 0xff;
    return out;
  }
  function asciiDecode(bytes) { var s = ""; for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i] & 0x7f); return s; }
  function latin1Decode(bytes) { var s = ""; for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]); return s; }

  function fromString(str, enc) {
    var e = normalizeEncoding(enc) || "utf8";
    var u8;
    switch (e) {
      case "utf8": u8 = utf8Encode(str); break;
      case "ascii": u8 = asciiEncode(str); break;
      case "latin1": u8 = latin1Encode(str); break;
      case "hex": u8 = hexDecode(str); break;
      case "base64": u8 = b64Decode(str.replace(/[^A-Za-z0-9+/=]/g, "")); break;
      case "base64url": u8 = b64Decode(base64UrlToStd(str.replace(/[^A-Za-z0-9\-_=]/g, ""))); break;
      case "utf16le": u8 = utf16leEncode(str); break;
      default: throw new TypeError("Unknown encoding: " + enc);
    }
    return wrap(u8);
  }

  // We splice the Buffer "constructor" onto Uint8Array's prototype chain so
  // result instanceof Uint8Array AND instanceof Buffer.
  function Buffer(arg, encOrOffset, length) {
    // Legacy: `new Buffer(...)` — Node deprecates this but many libs still do it.
    if (typeof arg === "number") return Buffer.alloc(arg);
    return Buffer.from(arg, encOrOffset, length);
  }

  // wrap a Uint8Array as Buffer (mutates prototype).
  function wrap(u8) {
    Object.setPrototypeOf(u8, Buffer.prototype);
    return u8;
  }
  Buffer.prototype = Object.create(Uint8Array.prototype);
  Buffer.prototype.constructor = Buffer;

  Buffer.isBuffer = function (b) {
    return b != null && b instanceof Uint8Array && Object.getPrototypeOf(b) === Buffer.prototype;
  };
  Buffer.isEncoding = isEncoding;
  Buffer.byteLength = function (str, enc) {
    if (str instanceof Uint8Array || str instanceof ArrayBuffer) return str.byteLength;
    var e = normalizeEncoding(enc) || "utf8";
    var s = String(str);
    switch (e) {
      case "utf8": return utf8ByteLength(s);
      case "ascii": case "latin1": return s.length;
      case "hex": return Math.floor(s.length / 2);
      case "base64": case "base64url":
        var n = s.replace(/=+$/, "").length;
        return Math.floor(n * 3 / 4);
      case "utf16le": return s.length * 2;
      default: return utf8ByteLength(s);
    }
  };
  Buffer.alloc = function (size, fill, encoding) {
    var b = wrap(new Uint8Array(size | 0));
    if (fill !== undefined && fill !== 0) b.fill(fill, 0, b.length, encoding);
    return b;
  };
  Buffer.allocUnsafe = function (size) { return wrap(new Uint8Array(size | 0)); };
  Buffer.allocUnsafeSlow = Buffer.allocUnsafe;
  Buffer.from = function (value, encOrOffset, length) {
    if (value == null) throw new TypeError("Buffer.from: input must not be null");
    if (typeof value === "string") return fromString(value, encOrOffset);
    if (value instanceof ArrayBuffer) {
      var off = encOrOffset | 0;
      var len = length === undefined ? value.byteLength - off : length | 0;
      return wrap(new Uint8Array(value, off, len));
    }
    if (ArrayBuffer.isView(value)) {
      return wrap(new Uint8Array(value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength)));
    }
    if (Array.isArray(value) || (typeof value === "object" && typeof value.length === "number")) {
      var arr = new Uint8Array(value.length);
      for (var i = 0; i < value.length; i++) arr[i] = value[i] & 0xff;
      return wrap(arr);
    }
    if (typeof value === "object" && value.type === "Buffer" && Array.isArray(value.data)) {
      return Buffer.from(value.data);
    }
    throw new TypeError("Buffer.from: unsupported input type");
  };
  Buffer.concat = function (list, totalLength) {
    if (!Array.isArray(list)) throw new TypeError("Buffer.concat: list must be Array");
    if (list.length === 0) return Buffer.alloc(0);
    if (totalLength === undefined) {
      totalLength = 0;
      for (var i = 0; i < list.length; i++) totalLength += list[i].length;
    }
    var out = wrap(new Uint8Array(totalLength | 0));
    var pos = 0;
    for (var j = 0; j < list.length && pos < totalLength; j++) {
      var src = list[j];
      var n = Math.min(src.length, totalLength - pos);
      out.set(src.subarray(0, n), pos);
      pos += n;
    }
    return out;
  };
  Buffer.compare = function (a, b) {
    if (!(a instanceof Uint8Array) || !(b instanceof Uint8Array)) throw new TypeError("Buffer.compare: arguments must be Buffers");
    var len = Math.min(a.length, b.length);
    for (var i = 0; i < len; i++) {
      if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
    }
    return a.length === b.length ? 0 : a.length < b.length ? -1 : 1;
  };

  Buffer.prototype.toString = function (encoding, start, end) {
    var e = normalizeEncoding(encoding) || "utf8";
    if (start == null) start = 0;
    if (end == null) end = this.length;
    start = Math.max(0, start | 0);
    end = Math.min(this.length, end | 0);
    if (end <= start) return "";
    var sub = this.subarray(start, end);
    switch (e) {
      case "utf8": return utf8Decode(sub);
      case "ascii": return asciiDecode(sub);
      case "latin1": return latin1Decode(sub);
      case "hex": return hexEncode(sub);
      case "base64": return b64Encode(sub);
      case "base64url": return stdToBase64Url(b64Encode(sub));
      case "utf16le": return utf16leDecode(sub);
      default: throw new TypeError("Unknown encoding: " + encoding);
    }
  };
  Buffer.prototype.toJSON = function () {
    var data = new Array(this.length);
    for (var i = 0; i < this.length; i++) data[i] = this[i];
    return { type: "Buffer", data: data };
  };
  Buffer.prototype.equals = function (other) { return Buffer.compare(this, other) === 0; };
  Buffer.prototype.compare = function (other) { return Buffer.compare(this, other); };
  Buffer.prototype.copy = function (target, targetStart, sourceStart, sourceEnd) {
    targetStart = targetStart | 0;
    sourceStart = sourceStart | 0;
    sourceEnd = sourceEnd === undefined ? this.length : sourceEnd | 0;
    if (sourceEnd > this.length) sourceEnd = this.length;
    var len = Math.min(sourceEnd - sourceStart, target.length - targetStart);
    if (len <= 0) return 0;
    target.set(this.subarray(sourceStart, sourceStart + len), targetStart);
    return len;
  };
  Buffer.prototype.slice = function (start, end) {
    if (start == null) start = 0;
    if (end == null) end = this.length;
    if (start < 0) start = Math.max(this.length + start, 0);
    if (end < 0) end = Math.max(this.length + end, 0);
    return wrap(new Uint8Array(this.buffer, this.byteOffset + start, Math.max(0, end - start)));
  };
  // Node 4+: subarray returns a Buffer
  Buffer.prototype.subarray = function (start, end) {
    var u = Uint8Array.prototype.subarray.call(this, start, end);
    Object.setPrototypeOf(u, Buffer.prototype);
    return u;
  };
  Buffer.prototype.write = function (str, offset, length, encoding) {
    if (typeof offset === "string") { encoding = offset; offset = 0; length = this.length; }
    else if (typeof length === "string") { encoding = length; length = this.length - (offset | 0); }
    offset = offset | 0;
    if (length === undefined || length > this.length - offset) length = this.length - offset;
    var src = fromString(str, encoding);
    var n = Math.min(src.length, length);
    this.set(src.subarray(0, n), offset);
    return n;
  };
  Buffer.prototype.fill = function (value, start, end, encoding) {
    if (start == null) start = 0;
    if (end == null) end = this.length;
    if (typeof value === "string") {
      var src = fromString(value, encoding);
      if (src.length === 0) return this;
      for (var i = start; i < end; i++) this[i] = src[(i - start) % src.length];
    } else if (typeof value === "number") {
      for (var j = start; j < end; j++) this[j] = value & 0xff;
    } else if (value instanceof Uint8Array) {
      for (var k = start; k < end; k++) this[k] = value[(k - start) % value.length];
    }
    return this;
  };
  Buffer.prototype.indexOf = function (value, byteOffset, encoding) {
    if (typeof byteOffset === "string") { encoding = byteOffset; byteOffset = 0; }
    byteOffset = byteOffset | 0;
    if (byteOffset < 0) byteOffset = Math.max(this.length + byteOffset, 0);
    var needle;
    if (typeof value === "number") needle = new Uint8Array([value & 0xff]);
    else if (typeof value === "string") needle = fromString(value, encoding);
    else if (value instanceof Uint8Array) needle = value;
    else return -1;
    if (needle.length === 0) return byteOffset;
    outer: for (var i = byteOffset; i <= this.length - needle.length; i++) {
      for (var j = 0; j < needle.length; j++) if (this[i + j] !== needle[j]) continue outer;
      return i;
    }
    return -1;
  };
  Buffer.prototype.lastIndexOf = function (value, byteOffset, encoding) {
    if (typeof byteOffset === "string") { encoding = byteOffset; byteOffset = this.length; }
    if (byteOffset === undefined || byteOffset > this.length) byteOffset = this.length;
    if (byteOffset < 0) byteOffset = Math.max(this.length + byteOffset, 0);
    var needle;
    if (typeof value === "number") needle = new Uint8Array([value & 0xff]);
    else if (typeof value === "string") needle = fromString(value, encoding);
    else if (value instanceof Uint8Array) needle = value;
    else return -1;
    if (needle.length === 0) return byteOffset;
    var start = Math.min(byteOffset, this.length - needle.length);
    outer: for (var i = start; i >= 0; i--) {
      for (var j = 0; j < needle.length; j++) if (this[i + j] !== needle[j]) continue outer;
      return i;
    }
    return -1;
  };
  Buffer.prototype.includes = function (value, byteOffset, encoding) { return this.indexOf(value, byteOffset, encoding) !== -1; };

  // --- numeric reads / writes (most-used variants) -----------------------
  function dvOf(b) { return new DataView(b.buffer, b.byteOffset, b.byteLength); }
  function defReadWrite(name, len, isLE, type) {
    Buffer.prototype["read" + name] = function (off, _noAssert) {
      off = off | 0;
      var dv = dvOf(this);
      switch (type) {
        case "u8": return dv.getUint8(off);
        case "i8": return dv.getInt8(off);
        case "u16": return dv.getUint16(off, isLE);
        case "i16": return dv.getInt16(off, isLE);
        case "u32": return dv.getUint32(off, isLE);
        case "i32": return dv.getInt32(off, isLE);
        case "f32": return dv.getFloat32(off, isLE);
        case "f64": return dv.getFloat64(off, isLE);
        case "bi64": return dv.getBigInt64(off, isLE);
        case "bu64": return dv.getBigUint64(off, isLE);
      }
    };
    Buffer.prototype["write" + name] = function (val, off, _noAssert) {
      off = off | 0;
      var dv = dvOf(this);
      switch (type) {
        case "u8": dv.setUint8(off, val); return off + 1;
        case "i8": dv.setInt8(off, val); return off + 1;
        case "u16": dv.setUint16(off, val, isLE); return off + 2;
        case "i16": dv.setInt16(off, val, isLE); return off + 2;
        case "u32": dv.setUint32(off, val, isLE); return off + 4;
        case "i32": dv.setInt32(off, val, isLE); return off + 4;
        case "f32": dv.setFloat32(off, val, isLE); return off + 4;
        case "f64": dv.setFloat64(off, val, isLE); return off + 8;
        case "bi64": dv.setBigInt64(off, val, isLE); return off + 8;
        case "bu64": dv.setBigUint64(off, val, isLE); return off + 8;
      }
      return off + len;
    };
  }
  defReadWrite("UInt8", 1, true, "u8");
  defReadWrite("Int8", 1, true, "i8");
  defReadWrite("UInt16LE", 2, true, "u16");  defReadWrite("UInt16BE", 2, false, "u16");
  defReadWrite("Int16LE", 2, true, "i16");   defReadWrite("Int16BE", 2, false, "i16");
  defReadWrite("UInt32LE", 4, true, "u32");  defReadWrite("UInt32BE", 4, false, "u32");
  defReadWrite("Int32LE", 4, true, "i32");   defReadWrite("Int32BE", 4, false, "i32");
  defReadWrite("FloatLE", 4, true, "f32");   defReadWrite("FloatBE", 4, false, "f32");
  defReadWrite("DoubleLE", 8, true, "f64");  defReadWrite("DoubleBE", 8, false, "f64");
  if (typeof BigInt64Array !== "undefined") {
    defReadWrite("BigInt64LE", 8, true, "bi64"); defReadWrite("BigInt64BE", 8, false, "bi64");
    defReadWrite("BigUInt64LE", 8, true, "bu64"); defReadWrite("BigUInt64BE", 8, false, "bu64");
  }

  Buffer.prototype.swap16 = function () {
    if (this.length % 2 !== 0) throw new RangeError("Buffer size must be a multiple of 16-bits");
    for (var i = 0; i < this.length; i += 2) { var t = this[i]; this[i] = this[i + 1]; this[i + 1] = t; }
    return this;
  };
  Buffer.prototype.swap32 = function () {
    if (this.length % 4 !== 0) throw new RangeError("Buffer size must be a multiple of 32-bits");
    for (var i = 0; i < this.length; i += 4) {
      var t = this[i]; this[i] = this[i + 3]; this[i + 3] = t;
      t = this[i + 1]; this[i + 1] = this[i + 2]; this[i + 2] = t;
    }
    return this;
  };
  Buffer.prototype.swap64 = function () {
    if (this.length % 8 !== 0) throw new RangeError("Buffer size must be a multiple of 64-bits");
    for (var i = 0; i < this.length; i += 8) {
      for (var k = 0; k < 4; k++) { var t = this[i + k]; this[i + k] = this[i + 7 - k]; this[i + 7 - k] = t; }
    }
    return this;
  };
  Buffer.prototype.inspect = function () { return "<Buffer " + hexEncode(this.subarray(0, Math.min(50, this.length))) + (this.length > 50 ? " ... " : "") + ">"; };

  // node:buffer module surface
  var bufferModule = {
    Buffer: Buffer,
    constants: { MAX_LENGTH: 4294967296, MAX_STRING_LENGTH: 536870888 },
    kMaxLength: 4294967296,
    kStringMaxLength: 536870888,
    INSPECT_MAX_BYTES: 50,
    Blob: globalThis.Blob,
    File: globalThis.File,
    atob: globalThis.atob,
    btoa: globalThis.btoa,
    isAscii: function (b) { for (var i = 0; i < b.length; i++) if (b[i] > 0x7f) return false; return true; },
    isUtf8: function (_b) { return true; }, // best-effort
    transcode: function (_src, _from, _to) { throw new Error("buffer.transcode: not supported in tynd-lite"); },
    resolveObjectURL: function () { throw new Error("buffer.resolveObjectURL: not supported in tynd-lite"); },
    SlowBuffer: Buffer,
  };

  globalThis.Buffer = Buffer;
  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["buffer"] = bufferModule;
  globalThis.__tynd_modules["node:buffer"] = bufferModule;
})();
