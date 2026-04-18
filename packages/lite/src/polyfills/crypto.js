// crypto.getRandomValues / randomUUID / subtle.digest / sign / verify / importKey
// (HMAC only). Web-standard surface. AES / RSA / ECDSA encryption and
// password hashing / KDFs are intentionally not polyfilled — ship a pure-JS
// lib (@noble/ciphers, @noble/hashes, hash-wasm) in userland if you need
// them. See ALTERNATIVES.md.
(function () {
  function bytesToBase64(u8) {
    var s = "";
    for (var i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]);
    return btoa(s);
  }
  function base64ToBytes(b64) {
    var s = atob(b64);
    var u8 = new Uint8Array(s.length);
    for (var i = 0; i < s.length; i++) u8[i] = s.charCodeAt(i);
    return u8;
  }
  function toBytes(data) {
    if (data instanceof Uint8Array) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (data && data.buffer instanceof ArrayBuffer) {
      return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    }
    throw new TypeError("crypto: expected BufferSource");
  }

  function getRandomValues(view) {
    if (!view || typeof view.byteLength !== "number") {
      throw new TypeError("getRandomValues: expected TypedArray");
    }
    if (view.byteLength > 65536) {
      throw new Error("getRandomValues: byteLength exceeds 65536");
    }
    var bytes = base64ToBytes(__tynd_crypto_random__(view.byteLength));
    var u8 = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
    u8.set(bytes);
    return view;
  }

  function randomUUID() {
    var b = new Uint8Array(16);
    getRandomValues(b);
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // variant
    var hex = "";
    for (var i = 0; i < 16; i++) {
      var h = b[i].toString(16);
      if (h.length === 1) h = "0" + h;
      hex += h;
    }
    return hex.slice(0, 8) + "-" + hex.slice(8, 12) + "-"
      + hex.slice(12, 16) + "-" + hex.slice(16, 20) + "-" + hex.slice(20, 32);
  }

  function digest(algo, data) {
    var name = (typeof algo === "string" ? algo : (algo && algo.name) || "").toUpperCase();
    var map = { "SHA-256": "sha256", "SHA-384": "sha384", "SHA-512": "sha512" };
    var rustAlgo = map[name];
    if (!rustAlgo) {
      return Promise.reject(new Error("crypto.subtle.digest: unsupported algo '" + name + "'"));
    }
    try {
      var out = base64ToBytes(__tynd_crypto_digest__(rustAlgo, bytesToBase64(toBytes(data))));
      var ab = new ArrayBuffer(out.length);
      new Uint8Array(ab).set(out);
      return Promise.resolve(ab);
    } catch (e) { return Promise.reject(e); }
  }

  function hmacAlgoName(key) {
    if (typeof key === "string") return key.replace(/-/g, "").toLowerCase();
    if (key && key.algorithm && key.algorithm.hash) {
      var h = key.algorithm.hash.name || key.algorithm.hash;
      return String(h).replace(/-/g, "").toLowerCase();
    }
    return "sha256";
  }

  function sign(algo, key, data) {
    // algo can be { name: "HMAC" } or "HMAC"; we only support HMAC here.
    var name = (typeof algo === "string" ? algo : (algo && algo.name) || "").toUpperCase();
    if (name !== "HMAC") {
      return Promise.reject(new Error("crypto.subtle.sign: only HMAC is supported on lite (see ALTERNATIVES.md for RSA/ECDSA)"));
    }
    var keyBytes = key && key._rawKey instanceof Uint8Array ? key._rawKey : toBytes(key);
    try {
      var sig64 = __tynd_crypto_hmac_sign__(hmacAlgoName(key), bytesToBase64(keyBytes), bytesToBase64(toBytes(data)));
      if (!sig64) return Promise.reject(new Error("crypto.subtle.sign failed"));
      var out = base64ToBytes(sig64);
      var ab = new ArrayBuffer(out.length);
      new Uint8Array(ab).set(out);
      return Promise.resolve(ab);
    } catch (e) { return Promise.reject(e); }
  }
  function verify(algo, key, signature, data) {
    var name = (typeof algo === "string" ? algo : (algo && algo.name) || "").toUpperCase();
    if (name !== "HMAC") {
      return Promise.reject(new Error("crypto.subtle.verify: only HMAC is supported on lite"));
    }
    var keyBytes = key && key._rawKey instanceof Uint8Array ? key._rawKey : toBytes(key);
    try {
      return Promise.resolve(__tynd_crypto_hmac_verify__(
        hmacAlgoName(key),
        bytesToBase64(keyBytes),
        bytesToBase64(toBytes(signature)),
        bytesToBase64(toBytes(data)),
      ));
    } catch (e) { return Promise.reject(e); }
  }

  function importKey(format, keyData, algorithm, _extractable, _usages) {
    if (format !== "raw") {
      return Promise.reject(new Error("crypto.subtle.importKey: only 'raw' format supported"));
    }
    var name = String((algorithm && (algorithm.name || algorithm)) || "HMAC").toUpperCase();
    if (name !== "HMAC") {
      return Promise.reject(new Error("crypto.subtle.importKey: only HMAC keys supported on lite"));
    }
    var hash = algorithm && algorithm.hash ? (algorithm.hash.name || algorithm.hash) : "SHA-256";
    return Promise.resolve({
      type: "secret",
      algorithm: { name: "HMAC", hash: { name: String(hash).toUpperCase() } },
      _rawKey: toBytes(keyData),
    });
  }

  globalThis.crypto = Object.freeze({
    getRandomValues: getRandomValues,
    randomUUID: randomUUID,
    subtle: Object.freeze({
      digest: digest,
      sign: sign,
      verify: verify,
      importKey: importKey,
    }),
  });
})();
