// node:os — system metadata. Backed by tiny Rust bridges
// (see polyfills::process_rs::register_os) which forward to
// `tynd_host::os::os_info` so we share the same impl as `os.*` calls
// from `@tynd/core/client`.
//
// Bridges:
//   __tynd_os_homedir__()  -> string|null
//   __tynd_os_tmpdir__()   -> string
//   __tynd_os_hostname__() -> string|null
//   __tynd_os_type__()     -> "Linux" | "Darwin" | "Windows_NT"
//   __tynd_os_release__()  -> string
//   __tynd_os_cpus__()     -> number  (logical CPU count)
//   __tynd_os_totalmem__() -> number  (bytes)
//   __tynd_os_freemem__()  -> number  (bytes)
//   __tynd_os_uptime__()   -> number  (seconds)
//   __tynd_os_loadavg__()  -> [number, number, number]
//   __tynd_os_networkInterfaces__() -> object
(function () {
  function call(fn, fb) { try { return typeof fn === "function" ? fn() : fb; } catch (_) { return fb; } }

  var EOL = (globalThis.__tynd_platform__ && __tynd_platform__() === "win32") ? "\r\n" : "\n";

  var os = {
    EOL: EOL,
    arch: function () { return globalThis.__tynd_arch__ ? __tynd_arch__() : "x64"; },
    platform: function () { return globalThis.__tynd_platform__ ? __tynd_platform__() : "linux"; },
    type: function () { return call(globalThis.__tynd_os_type__, "Linux"); },
    release: function () { return call(globalThis.__tynd_os_release__, "0.0.0"); },
    version: function () { return call(globalThis.__tynd_os_release__, "0.0.0"); },
    homedir: function () { return call(globalThis.__tynd_os_homedir__, "/"); },
    tmpdir: function () { return call(globalThis.__tynd_os_tmpdir__, "/tmp"); },
    hostname: function () { return call(globalThis.__tynd_os_hostname__, "localhost"); },
    cpus: function () {
      // Build N stub objects so `os.cpus().length` is the expected count.
      // Per-core stats not exposed (tynd_host doesn't snapshot per-core).
      var n = call(globalThis.__tynd_os_cpus__, 1) | 0;
      if (n < 1) n = 1;
      var out = [];
      for (var i = 0; i < n; i++) {
        out.push({ model: "unknown", speed: 0, times: { user: 0, nice: 0, sys: 0, idle: 0, irq: 0 } });
      }
      return out;
    },
    totalmem: function () { return call(globalThis.__tynd_os_totalmem__, 0); },
    freemem: function () { return call(globalThis.__tynd_os_freemem__, 0); },
    uptime: function () { return call(globalThis.__tynd_os_uptime__, 0); },
    loadavg: function () {
      var v = call(globalThis.__tynd_os_loadavg__, null);
      return Array.isArray(v) ? v : [0, 0, 0];
    },
    networkInterfaces: function () {
      var v = call(globalThis.__tynd_os_networkInterfaces__, null);
      return v && typeof v === "object" ? v : {};
    },
    userInfo: function () {
      return {
        uid: -1, gid: -1, shell: null,
        username: (globalThis.process && process.env && process.env.USER) || "tynd",
        homedir: call(globalThis.__tynd_os_homedir__, "/"),
      };
    },
    endianness: function () {
      var b = new ArrayBuffer(2);
      new DataView(b).setUint16(0, 256, true);
      return new Uint8Array(b)[0] === 0 ? "BE" : "LE";
    },
    constants: {
      signals: { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGTERM: 15, SIGKILL: 9 },
      errno: { EPERM: 1, ENOENT: 2, EINTR: 4, EIO: 5, EBADF: 9, EAGAIN: 11, ENOMEM: 12, EACCES: 13, EBUSY: 16, EEXIST: 17, ENOTDIR: 20, EISDIR: 21, EINVAL: 22, EMFILE: 24, EPIPE: 32, ENOTEMPTY: 39, ETIMEDOUT: 110 },
      priority: { PRIORITY_LOW: 19, PRIORITY_BELOW_NORMAL: 10, PRIORITY_NORMAL: 0, PRIORITY_ABOVE_NORMAL: -7, PRIORITY_HIGH: -14, PRIORITY_HIGHEST: -20 },
    },
    devNull: (globalThis.__tynd_platform__ && __tynd_platform__() === "win32") ? "\\\\.\\nul" : "/dev/null",
    machine: function () { return globalThis.__tynd_arch__ ? __tynd_arch__() : "x86_64"; },
    setPriority: function () {},
    getPriority: function () { return 0; },
    availableParallelism: function () {
      var n = call(globalThis.__tynd_os_cpus__, 1) | 0;
      return n < 1 ? 1 : n;
    },
  };

  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["os"] = os;
  globalThis.__tynd_modules["node:os"] = os;
})();
