// Blob / File / FormData — pure-JS, backed by an internal Uint8Array blob.
// Covers ~95% of practical usage: size, type, text(), arrayBuffer(), bytes(),
// slice(), stream() returning a ReadableStream compatible with our polyfill.
(function () {
  function normalizeParts(parts) {
    var chunks = [];
    var total = 0;
    for (var i = 0; i < parts.length; i++) {
      var p = parts[i];
      var u8;
      if (typeof p === "string") {
        u8 = new TextEncoder().encode(p);
      } else if (p instanceof ArrayBuffer) {
        u8 = new Uint8Array(p);
      } else if (p instanceof Uint8Array) {
        u8 = new Uint8Array(p.buffer, p.byteOffset, p.byteLength);
      } else if (p && p._tyndBlobBytes instanceof Uint8Array) {
        u8 = p._tyndBlobBytes;
      } else if (ArrayBuffer.isView(p)) {
        u8 = new Uint8Array(p.buffer, p.byteOffset, p.byteLength);
      } else {
        u8 = new TextEncoder().encode(String(p));
      }
      chunks.push(u8);
      total += u8.length;
    }
    var out = new Uint8Array(total);
    var off = 0;
    for (var j = 0; j < chunks.length; j++) { out.set(chunks[j], off); off += chunks[j].length; }
    return out;
  }

  function Blob(parts, options) {
    this._tyndBlobBytes = parts ? normalizeParts(parts) : new Uint8Array(0);
    this.size = this._tyndBlobBytes.length;
    this.type = (options && options.type) || "";
  }
  Blob.prototype.arrayBuffer = function () {
    var u8 = this._tyndBlobBytes;
    var ab = new ArrayBuffer(u8.length);
    new Uint8Array(ab).set(u8);
    return Promise.resolve(ab);
  };
  Blob.prototype.bytes = function () {
    return Promise.resolve(new Uint8Array(this._tyndBlobBytes));
  };
  Blob.prototype.text = function () {
    return Promise.resolve(new TextDecoder().decode(this._tyndBlobBytes));
  };
  Blob.prototype.slice = function (start, end, type) {
    var u8 = this._tyndBlobBytes.subarray(start || 0, end == null ? this._tyndBlobBytes.length : end);
    var b = new Blob();
    b._tyndBlobBytes = new Uint8Array(u8); // copy
    b.size = b._tyndBlobBytes.length;
    b.type = type || "";
    return b;
  };
  Blob.prototype.stream = function () {
    var u8 = this._tyndBlobBytes;
    return new ReadableStream({
      start: function (c) { c.enqueue(new Uint8Array(u8)); c.close(); },
    });
  };

  function File(parts, name, options) {
    Blob.call(this, parts, options);
    this.name = String(name);
    this.lastModified = (options && options.lastModified) || Date.now();
  }
  File.prototype = Object.create(Blob.prototype);
  File.prototype.constructor = File;

  function FormData() { this._entries = []; }
  FormData.prototype.append = function (name, value, filename) {
    this._entries.push([String(name), toEntry(value, filename)]);
  };
  FormData.prototype.set = function (name, value, filename) {
    name = String(name);
    var replaced = false;
    this._entries = this._entries.filter(function (e) {
      if (e[0] !== name) return true;
      if (!replaced) { e[1] = toEntry(value, filename); replaced = true; return true; }
      return false;
    });
    if (!replaced) this._entries.push([name, toEntry(value, filename)]);
  };
  FormData.prototype.delete = function (name) {
    name = String(name);
    this._entries = this._entries.filter(function (e) { return e[0] !== name; });
  };
  FormData.prototype.get = function (name) {
    name = String(name);
    for (var i = 0; i < this._entries.length; i++) if (this._entries[i][0] === name) return this._entries[i][1];
    return null;
  };
  FormData.prototype.getAll = function (name) {
    name = String(name);
    return this._entries.filter(function (e) { return e[0] === name; }).map(function (e) { return e[1]; });
  };
  FormData.prototype.has = function (name) {
    name = String(name);
    for (var i = 0; i < this._entries.length; i++) if (this._entries[i][0] === name) return true;
    return false;
  };
  FormData.prototype.forEach = function (cb, self) {
    for (var i = 0; i < this._entries.length; i++) cb.call(self, this._entries[i][1], this._entries[i][0], this);
  };
  FormData.prototype.entries = function* () {
    for (var i = 0; i < this._entries.length; i++) yield [this._entries[i][0], this._entries[i][1]];
  };
  FormData.prototype.keys = function* () {
    for (var i = 0; i < this._entries.length; i++) yield this._entries[i][0];
  };
  FormData.prototype.values = function* () {
    for (var i = 0; i < this._entries.length; i++) yield this._entries[i][1];
  };
  FormData.prototype[Symbol.iterator] = FormData.prototype.entries;

  function toEntry(value, filename) {
    if (value instanceof Blob) {
      if (filename && !(value instanceof File)) return new File([value._tyndBlobBytes], filename, { type: value.type });
      return value;
    }
    return String(value);
  }

  globalThis.Blob = Blob;
  globalThis.File = File;
  globalThis.FormData = FormData;
})();
