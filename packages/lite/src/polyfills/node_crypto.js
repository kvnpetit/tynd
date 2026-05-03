// node:crypto — Node-style additions (`createHash` / `createHmac` /
// `randomBytes` / `randomUUID` / `randomInt` / `timingSafeEqual` /
// `getRandomValues` / `pbkdf2`/`scrypt` reject helpfully).
//
// Backed by the same Rust bridges as crypto.subtle:
//   __tynd_crypto_digest__(algo, b64)
//   __tynd_crypto_hmac_sign__(algo, keyB64, dataB64)
//   __tynd_crypto_random__(n)
//   __tynd_random_uuid__()
(function () {
  var b64Encode = globalThis.__tynd_b64_encode;
  var b64Decode = globalThis.__tynd_b64_decode;
  var Buffer = globalThis.Buffer;

  function normAlgo(name) {
    var n = String(name).toLowerCase().replace(/-/g, "");
    return { sha256: "sha256", sha384: "sha384", sha512: "sha512" }[n] || null;
  }

  function Hash(algo) {
    this._algo = normAlgo(algo);
    if (!this._algo) throw new Error("crypto.createHash: unsupported algorithm '" + algo + "' (lite supports sha256/384/512). Use @noble/hashes for md5/sha1.");
    this._chunks = [];
  }
  Hash.prototype.update = function (data, encoding) {
    var b = typeof data === "string" ? Buffer.from(data, encoding || "utf8") : (data instanceof Uint8Array ? data : Buffer.from(data));
    this._chunks.push(b);
    return this;
  };
  Hash.prototype.digest = function (encoding) {
    var concat = Buffer.concat(this._chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); }));
    var resB64 = __tynd_crypto_digest__(this._algo, b64Encode(concat));
    var bytes = b64Decode(resB64);
    var buf = Buffer.from(bytes);
    return encoding ? buf.toString(encoding) : buf;
  };
  Hash.prototype.copy = function () { var h = new Hash(this._algo); h._chunks = this._chunks.slice(); return h; };

  function Hmac(algo, key) {
    this._algo = normAlgo(algo);
    if (!this._algo) throw new Error("crypto.createHmac: unsupported algorithm '" + algo + "' (lite supports sha256/384/512)");
    this._key = key instanceof Uint8Array ? key : (typeof key === "string" ? Buffer.from(key, "utf8") : Buffer.from(key));
    this._chunks = [];
  }
  Hmac.prototype.update = function (data, encoding) {
    var b = typeof data === "string" ? Buffer.from(data, encoding || "utf8") : (data instanceof Uint8Array ? data : Buffer.from(data));
    this._chunks.push(b);
    return this;
  };
  Hmac.prototype.digest = function (encoding) {
    var concat = Buffer.concat(this._chunks.map(function (c) { return c instanceof Uint8Array ? c : Buffer.from(c); }));
    var sigB64 = __tynd_crypto_hmac_sign__(this._algo, b64Encode(this._key), b64Encode(concat));
    var bytes = b64Decode(sigB64);
    var buf = Buffer.from(bytes);
    return encoding ? buf.toString(encoding) : buf;
  };

  function randomBytes(size, cb) {
    var n = size | 0;
    if (n > 65536) {
      // chunk to bypass the 64KiB getRandomValues cap
      var out = Buffer.alloc(n);
      var off = 0;
      while (off < n) {
        var chunk = Math.min(65536, n - off);
        out.set(b64Decode(__tynd_crypto_random__(chunk)).subarray(0, chunk), off);
        off += chunk;
      }
      if (cb) { cb(null, out); return; }
      return out;
    }
    var bytes = b64Decode(__tynd_crypto_random__(n));
    var buf = Buffer.from(bytes);
    if (cb) { cb(null, buf); return; }
    return buf;
  }
  function randomFillSync(view, offset, size) {
    offset = offset | 0;
    size = size === undefined ? view.byteLength - offset : size | 0;
    var u8 = view instanceof Uint8Array ? view : new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    var b = randomBytes(size);
    u8.set(b, offset);
    return view;
  }
  function randomFill(view, offset, size, cb) {
    if (typeof offset === "function") { cb = offset; offset = 0; size = view.byteLength; }
    else if (typeof size === "function") { cb = size; size = view.byteLength - (offset | 0); }
    try { var r = randomFillSync(view, offset, size); cb(null, r); }
    catch (e) { cb(e); }
  }

  function randomUUID() { return typeof __tynd_random_uuid__ === "function" ? __tynd_random_uuid__() : globalThis.crypto.randomUUID(); }
  function randomInt(min, max, cb) {
    if (max === undefined) { max = min; min = 0; }
    var range = max - min;
    var bytes = randomBytes(6);
    var n = min + (bytes.readUIntBE(0, 6) % range);
    if (cb) { cb(null, n); return; }
    return n;
  }
  function timingSafeEqual(a, b) {
    if (a.length !== b.length) return false;
    var diff = 0;
    for (var i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
    return diff === 0;
  }

  function unsupported(name, alt) {
    return function () {
      throw new Error(
        "crypto." + name + " is not supported in tynd-lite. " + (alt || "") +
        " See https://tynd.kvnpetit.com/v1.0/guides/native-plugins-mapping#crypto"
      );
    };
  }

  var nodeCrypto = {
    createHash: function (algo) { return new Hash(algo); },
    createHmac: function (algo, key) { return new Hmac(algo, key); },
    randomBytes: randomBytes,
    randomFillSync: randomFillSync, randomFill: randomFill,
    randomUUID: randomUUID, randomInt: randomInt,
    timingSafeEqual: timingSafeEqual,
    getRandomValues: function (v) { return globalThis.crypto.getRandomValues(v); },
    webcrypto: globalThis.crypto,
    subtle: globalThis.crypto.subtle,
    constants: {},
    Hash: Hash, Hmac: Hmac,

    createCipheriv: unsupported("createCipheriv", "Use @noble/ciphers (pure-JS, AES-GCM/CTR/CBC + ChaCha20-Poly1305)."),
    createDecipheriv: unsupported("createDecipheriv", "Use @noble/ciphers."),
    createSign: unsupported("createSign", "Use @noble/curves for Ed25519/secp256k1; @noble/hashes for digests."),
    createVerify: unsupported("createVerify", "Use @noble/curves."),
    pbkdf2: function (_p, _s, _i, _l, _d, cb) { cb(new Error("crypto.pbkdf2 not supported in lite — use @noble/hashes pbkdf2.")); },
    pbkdf2Sync: unsupported("pbkdf2Sync", "Use @noble/hashes pbkdf2."),
    scrypt: function (_p, _s, _l, _o, cb) { var fn = typeof _o === "function" ? _o : cb; fn(new Error("crypto.scrypt not supported in lite — use @noble/hashes scrypt.")); },
    scryptSync: unsupported("scryptSync", "Use @noble/hashes scrypt."),
    generateKeyPair: function (_t, _o, cb) { cb(new Error("crypto.generateKeyPair not supported in lite — use @noble/curves.")); },
    generateKeyPairSync: unsupported("generateKeyPairSync", "Use @noble/curves."),
    sign: unsupported("sign", "Use @noble/curves."),
    verify: unsupported("verify", "Use @noble/curves."),
  };

  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["crypto"] = nodeCrypto;
  globalThis.__tynd_modules["node:crypto"] = nodeCrypto;
})();
