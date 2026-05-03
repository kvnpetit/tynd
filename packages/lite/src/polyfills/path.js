// node:path — POSIX + Win32 implementations, exposed under
// `path` (current platform) plus explicit `path.posix` / `path.win32`.
//
// Pure JS port of Node's path semantics. Covers join, resolve, normalize,
// dirname, basename, extname, parse, format, isAbsolute, relative, sep, delimiter.
//
// Platform detection uses globalThis.__tynd_platform__ (set by Rust bridge,
// see polyfills::process). Falls back to posix if absent.
(function () {
  function assertString(p, name) {
    if (typeof p !== "string") {
      throw new TypeError("Path must be a string. Received " + typeof p + " for '" + name + "'");
    }
  }

  // ---- POSIX -----------------------------------------------------------
  function normalizeStringPosix(path, allowAboveRoot) {
    var res = "";
    var lastSegmentLength = 0;
    var lastSlash = -1;
    var dots = 0;
    var code;
    for (var i = 0; i <= path.length; ++i) {
      if (i < path.length) code = path.charCodeAt(i);
      else if (code === 47) break;
      else code = 47;
      if (code === 47) {
        if (lastSlash === i - 1 || dots === 1) { /* skip */ }
        else if (lastSlash !== i - 1 && dots === 2) {
          if (res.length < 2 || lastSegmentLength !== 2 ||
              res.charCodeAt(res.length - 1) !== 46 ||
              res.charCodeAt(res.length - 2) !== 46) {
            if (res.length > 2) {
              var lastSlashIndex = res.lastIndexOf("/");
              if (lastSlashIndex !== res.length - 1) {
                if (lastSlashIndex === -1) { res = ""; lastSegmentLength = 0; }
                else {
                  res = res.slice(0, lastSlashIndex);
                  lastSegmentLength = res.length - 1 - res.lastIndexOf("/");
                }
                lastSlash = i;
                dots = 0;
                continue;
              }
            } else if (res.length === 2 || res.length === 1) {
              res = ""; lastSegmentLength = 0; lastSlash = i; dots = 0; continue;
            }
          }
          if (allowAboveRoot) {
            if (res.length > 0) res += "/..";
            else res = "..";
            lastSegmentLength = 2;
          }
        } else {
          if (res.length > 0) res += "/" + path.slice(lastSlash + 1, i);
          else res = path.slice(lastSlash + 1, i);
          lastSegmentLength = i - lastSlash - 1;
        }
        lastSlash = i; dots = 0;
      } else if (code === 46 && dots !== -1) dots++;
      else dots = -1;
    }
    return res;
  }

  var posix = {
    sep: "/",
    delimiter: ":",
    resolve: function () {
      var resolvedPath = "";
      var resolvedAbsolute = false;
      for (var i = arguments.length - 1; i >= -1 && !resolvedAbsolute; i--) {
        var path = i >= 0 ? arguments[i] : posixCwd();
        assertString(path, "path");
        if (path.length === 0) continue;
        resolvedPath = path + "/" + resolvedPath;
        resolvedAbsolute = path.charCodeAt(0) === 47;
      }
      resolvedPath = normalizeStringPosix(resolvedPath, !resolvedAbsolute);
      if (resolvedAbsolute) return "/" + resolvedPath;
      return resolvedPath.length > 0 ? resolvedPath : ".";
    },
    normalize: function (path) {
      assertString(path, "path");
      if (path.length === 0) return ".";
      var isAbs = path.charCodeAt(0) === 47;
      var trailingSep = path.charCodeAt(path.length - 1) === 47;
      path = normalizeStringPosix(path, !isAbs);
      if (path.length === 0 && !isAbs) path = ".";
      if (path.length > 0 && trailingSep) path += "/";
      return isAbs ? "/" + path : path;
    },
    isAbsolute: function (path) {
      assertString(path, "path");
      return path.length > 0 && path.charCodeAt(0) === 47;
    },
    join: function () {
      if (arguments.length === 0) return ".";
      var joined;
      for (var i = 0; i < arguments.length; ++i) {
        var arg = arguments[i];
        assertString(arg, "path");
        if (arg.length > 0) joined = joined === undefined ? arg : joined + "/" + arg;
      }
      return joined === undefined ? "." : posix.normalize(joined);
    },
    relative: function (from, to) {
      assertString(from, "from"); assertString(to, "to");
      if (from === to) return "";
      from = posix.resolve(from); to = posix.resolve(to);
      if (from === to) return "";
      var fromStart = 1;
      for (; fromStart < from.length; ++fromStart) if (from.charCodeAt(fromStart) !== 47) break;
      var fromEnd = from.length, fromLen = fromEnd - fromStart;
      var toStart = 1;
      for (; toStart < to.length; ++toStart) if (to.charCodeAt(toStart) !== 47) break;
      var toEnd = to.length, toLen = toEnd - toStart;
      var length = fromLen < toLen ? fromLen : toLen;
      var lastCommonSep = -1, i2 = 0;
      for (; i2 <= length; ++i2) {
        if (i2 === length) {
          if (toLen > length) {
            if (to.charCodeAt(toStart + i2) === 47) return to.slice(toStart + i2 + 1);
            if (i2 === 0) return to.slice(toStart + i2);
          } else if (fromLen > length) {
            if (from.charCodeAt(fromStart + i2) === 47) lastCommonSep = i2;
            else if (i2 === 0) lastCommonSep = 0;
          }
          break;
        }
        var fromCode = from.charCodeAt(fromStart + i2);
        var toCode = to.charCodeAt(toStart + i2);
        if (fromCode !== toCode) break;
        else if (fromCode === 47) lastCommonSep = i2;
      }
      var out = "";
      for (i2 = fromStart + lastCommonSep + 1; i2 <= fromEnd; ++i2) {
        if (i2 === fromEnd || from.charCodeAt(i2) === 47) {
          if (out.length === 0) out += "..";
          else out += "/..";
        }
      }
      if (out.length > 0) return out + to.slice(toStart + lastCommonSep);
      toStart += lastCommonSep;
      if (to.charCodeAt(toStart) === 47) ++toStart;
      return to.slice(toStart);
    },
    dirname: function (path) {
      assertString(path, "path");
      if (path.length === 0) return ".";
      var hasRoot = path.charCodeAt(0) === 47;
      var end = -1, matchedSlash = true;
      for (var i = path.length - 1; i >= 1; --i) {
        if (path.charCodeAt(i) === 47) { if (!matchedSlash) { end = i; break; } }
        else { matchedSlash = false; }
      }
      if (end === -1) return hasRoot ? "/" : ".";
      if (hasRoot && end === 1) return "//";
      return path.slice(0, end);
    },
    basename: function (path, ext) {
      if (ext !== undefined && typeof ext !== "string") throw new TypeError('"ext" must be a string');
      assertString(path, "path");
      var start = 0, end = -1, matchedSlash = true, i;
      if (ext !== undefined && ext.length > 0 && ext.length <= path.length) {
        if (ext.length === path.length && ext === path) return "";
        var extIdx = ext.length - 1;
        var firstNonSlashEnd = -1;
        for (i = path.length - 1; i >= 0; --i) {
          var code = path.charCodeAt(i);
          if (code === 47) { if (!matchedSlash) { start = i + 1; break; } }
          else {
            if (firstNonSlashEnd === -1) { matchedSlash = false; firstNonSlashEnd = i + 1; }
            if (extIdx >= 0) {
              if (code === ext.charCodeAt(extIdx)) {
                if (--extIdx === -1) end = i;
              } else { extIdx = -1; end = firstNonSlashEnd; }
            }
          }
        }
        if (start === end) end = firstNonSlashEnd;
        else if (end === -1) end = path.length;
        return path.slice(start, end);
      }
      for (i = path.length - 1; i >= 0; --i) {
        if (path.charCodeAt(i) === 47) { if (!matchedSlash) { start = i + 1; break; } }
        else if (end === -1) { matchedSlash = false; end = i + 1; }
      }
      return end === -1 ? "" : path.slice(start, end);
    },
    extname: function (path) {
      assertString(path, "path");
      var startDot = -1, startPart = 0, end = -1, matchedSlash = true, preDotState = 0;
      for (var i = path.length - 1; i >= 0; --i) {
        var code = path.charCodeAt(i);
        if (code === 47) { if (!matchedSlash) { startPart = i + 1; break; } continue; }
        if (end === -1) { matchedSlash = false; end = i + 1; }
        if (code === 46) {
          if (startDot === -1) startDot = i;
          else if (preDotState !== 1) preDotState = 1;
        } else if (startDot !== -1) preDotState = -1;
      }
      if (startDot === -1 || end === -1 || preDotState === 0 ||
          (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)) return "";
      return path.slice(startDot, end);
    },
    format: function (pathObject) { return _format("/", pathObject); },
    parse: function (path) {
      assertString(path, "path");
      var ret = { root: "", dir: "", base: "", ext: "", name: "" };
      if (path.length === 0) return ret;
      var isAbs = path.charCodeAt(0) === 47, start;
      if (isAbs) { ret.root = "/"; start = 1; } else start = 0;
      var startDot = -1, startPart = 0, end = -1, matchedSlash = true, i = path.length - 1, preDotState = 0;
      for (; i >= start; --i) {
        var code = path.charCodeAt(i);
        if (code === 47) { if (!matchedSlash) { startPart = i + 1; break; } continue; }
        if (end === -1) { matchedSlash = false; end = i + 1; }
        if (code === 46) {
          if (startDot === -1) startDot = i;
          else if (preDotState !== 1) preDotState = 1;
        } else if (startDot !== -1) preDotState = -1;
      }
      if (startDot === -1 || end === -1 || preDotState === 0 ||
          (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)) {
        if (end !== -1) {
          if (startPart === 0 && isAbs) ret.base = ret.name = path.slice(1, end);
          else ret.base = ret.name = path.slice(startPart, end);
        }
      } else {
        if (startPart === 0 && isAbs) {
          ret.name = path.slice(1, startDot);
          ret.base = path.slice(1, end);
        } else {
          ret.name = path.slice(startPart, startDot);
          ret.base = path.slice(startPart, end);
        }
        ret.ext = path.slice(startDot, end);
      }
      if (startPart > 0) ret.dir = path.slice(0, startPart - 1);
      else if (isAbs) ret.dir = "/";
      return ret;
    },
    toNamespacedPath: function (p) { return p; },
  };
  posix.posix = posix;

  function posixCwd() {
    if (globalThis.__tynd_cwd__) {
      try { var c = globalThis.__tynd_cwd__(); if (typeof c === "string") return c.replace(/\\/g, "/"); } catch (_) {}
    }
    return "/";
  }

  // ---- WIN32 -----------------------------------------------------------
  function isPathSeparator(code) { return code === 47 || code === 92; }
  function isWindowsDeviceRoot(code) {
    return (code >= 65 && code <= 90) || (code >= 97 && code <= 122);
  }
  function normalizeStringWin32(path, allowAboveRoot) {
    var res = "", lastSegmentLength = 0, lastSlash = -1, dots = 0, code;
    for (var i = 0; i <= path.length; ++i) {
      if (i < path.length) code = path.charCodeAt(i);
      else if (isPathSeparator(code)) break;
      else code = 47;
      if (isPathSeparator(code)) {
        if (lastSlash === i - 1 || dots === 1) {}
        else if (lastSlash !== i - 1 && dots === 2) {
          if (res.length < 2 || lastSegmentLength !== 2 ||
              res.charCodeAt(res.length - 1) !== 46 ||
              res.charCodeAt(res.length - 2) !== 46) {
            if (res.length > 2) {
              var lastSlashIndex = res.lastIndexOf("\\");
              if (lastSlashIndex === -1) { res = ""; lastSegmentLength = 0; }
              else {
                res = res.slice(0, lastSlashIndex);
                lastSegmentLength = res.length - 1 - res.lastIndexOf("\\");
              }
              lastSlash = i; dots = 0; continue;
            } else if (res.length === 2 || res.length === 1) {
              res = ""; lastSegmentLength = 0; lastSlash = i; dots = 0; continue;
            }
          }
          if (allowAboveRoot) {
            if (res.length > 0) res += "\\..";
            else res = "..";
            lastSegmentLength = 2;
          }
        } else {
          if (res.length > 0) res += "\\" + path.slice(lastSlash + 1, i);
          else res = path.slice(lastSlash + 1, i);
          lastSegmentLength = i - lastSlash - 1;
        }
        lastSlash = i; dots = 0;
      } else if (code === 46 && dots !== -1) dots++;
      else dots = -1;
    }
    return res;
  }

  var win32 = {
    sep: "\\",
    delimiter: ";",
    resolve: function () {
      var resolvedDevice = "", resolvedTail = "", resolvedAbsolute = false;
      for (var i = arguments.length - 1; i >= -1; i--) {
        var path;
        if (i >= 0) path = arguments[i];
        else if (!resolvedDevice) path = posixCwd();
        else { path = posixCwd(); }
        assertString(path, "path");
        if (path.length === 0) continue;
        var len = path.length, rootEnd = 0, device = "", isAbsolute = false;
        var code = path.charCodeAt(0);
        if (len > 1) {
          if (isPathSeparator(code)) {
            isAbsolute = true;
            if (isPathSeparator(path.charCodeAt(1))) {
              var j = 2, last = j;
              for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
              if (j < len && j !== last) {
                var firstPart = path.slice(last, j);
                last = j;
                for (; j < len; ++j) if (!isPathSeparator(path.charCodeAt(j))) break;
                if (j < len && j !== last) {
                  last = j;
                  for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
                  if (j === len) { device = "\\\\" + firstPart + "\\" + path.slice(last); rootEnd = j; }
                  else if (j !== last) { device = "\\\\" + firstPart + "\\" + path.slice(last, j); rootEnd = j; }
                }
              }
            } else rootEnd = 1;
          } else if (isWindowsDeviceRoot(code) && path.charCodeAt(1) === 58) {
            device = path.slice(0, 2); rootEnd = 2;
            if (len > 2 && isPathSeparator(path.charCodeAt(2))) { isAbsolute = true; rootEnd = 3; }
          }
        } else if (isPathSeparator(code)) { rootEnd = 1; isAbsolute = true; }
        if (device.length > 0 && resolvedDevice.length > 0 && device.toLowerCase() !== resolvedDevice.toLowerCase()) continue;
        if (resolvedDevice.length === 0 && device.length > 0) resolvedDevice = device;
        if (!resolvedAbsolute) {
          resolvedTail = path.slice(rootEnd) + "\\" + resolvedTail;
          resolvedAbsolute = isAbsolute;
        }
        if (resolvedDevice.length > 0 && resolvedAbsolute) break;
      }
      resolvedTail = normalizeStringWin32(resolvedTail, !resolvedAbsolute);
      return resolvedDevice + (resolvedAbsolute ? "\\" : "") + resolvedTail || ".";
    },
    normalize: function (path) {
      assertString(path, "path");
      var len = path.length;
      if (len === 0) return ".";
      var rootEnd = 0, device, isAbsolute = false;
      var code = path.charCodeAt(0);
      if (len > 1) {
        if (isPathSeparator(code)) {
          isAbsolute = true;
          if (isPathSeparator(path.charCodeAt(1))) {
            var j = 2, last = j;
            for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
            if (j < len && j !== last) {
              var firstPart = path.slice(last, j);
              last = j;
              for (; j < len; ++j) if (!isPathSeparator(path.charCodeAt(j))) break;
              if (j < len && j !== last) {
                last = j;
                for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
                if (j === len) return "\\\\" + firstPart + "\\" + path.slice(last) + "\\";
                else if (j !== last) { device = "\\\\" + firstPart + "\\" + path.slice(last, j); rootEnd = j; }
              }
            }
          } else rootEnd = 1;
        } else if (isWindowsDeviceRoot(code) && path.charCodeAt(1) === 58) {
          device = path.slice(0, 2); rootEnd = 2;
          if (len > 2 && isPathSeparator(path.charCodeAt(2))) { isAbsolute = true; rootEnd = 3; }
        }
      } else if (isPathSeparator(code)) return "\\";
      var tail;
      if (rootEnd < len) tail = normalizeStringWin32(path.slice(rootEnd), !isAbsolute);
      else tail = "";
      if (tail.length === 0 && !isAbsolute) tail = ".";
      if (tail.length > 0 && isPathSeparator(path.charCodeAt(len - 1))) tail += "\\";
      if (device === undefined) return isAbsolute ? "\\" + tail : tail;
      return isAbsolute ? device + "\\" + tail : device + tail;
    },
    isAbsolute: function (path) {
      assertString(path, "path");
      var len = path.length;
      if (len === 0) return false;
      var code = path.charCodeAt(0);
      if (isPathSeparator(code)) return true;
      if (isWindowsDeviceRoot(code) && len > 2 && path.charCodeAt(1) === 58 && isPathSeparator(path.charCodeAt(2))) return true;
      return false;
    },
    join: function () {
      if (arguments.length === 0) return ".";
      var joined, firstPart;
      for (var i = 0; i < arguments.length; ++i) {
        var arg = arguments[i];
        assertString(arg, "path");
        if (arg.length > 0) {
          if (joined === undefined) joined = firstPart = arg;
          else joined += "\\" + arg;
        }
      }
      if (joined === undefined) return ".";
      var needsReplace = true, slashCount = 0;
      if (isPathSeparator(firstPart.charCodeAt(0))) {
        ++slashCount;
        var firstLen = firstPart.length;
        if (firstLen > 1 && isPathSeparator(firstPart.charCodeAt(1))) {
          ++slashCount;
          if (firstLen > 2) {
            if (isPathSeparator(firstPart.charCodeAt(2))) ++slashCount;
            else needsReplace = false;
          }
        }
      }
      if (needsReplace) {
        for (; slashCount < joined.length; ++slashCount) if (!isPathSeparator(joined.charCodeAt(slashCount))) break;
        if (slashCount >= 2) joined = "\\" + joined.slice(slashCount);
      }
      return win32.normalize(joined);
    },
    relative: function (from, to) {
      assertString(from, "from"); assertString(to, "to");
      if (from === to) return "";
      var fromOrig = win32.resolve(from), toOrig = win32.resolve(to);
      if (fromOrig === toOrig) return "";
      from = fromOrig.toLowerCase(); to = toOrig.toLowerCase();
      if (from === to) return "";
      var fromStart = 0;
      for (; fromStart < from.length; ++fromStart) if (from.charCodeAt(fromStart) !== 92) break;
      var fromEnd = from.length;
      for (; fromEnd - 1 > fromStart; --fromEnd) if (from.charCodeAt(fromEnd - 1) !== 92) break;
      var fromLen = fromEnd - fromStart;
      var toStart = 0;
      for (; toStart < to.length; ++toStart) if (to.charCodeAt(toStart) !== 92) break;
      var toEnd = to.length;
      for (; toEnd - 1 > toStart; --toEnd) if (to.charCodeAt(toEnd - 1) !== 92) break;
      var toLen = toEnd - toStart;
      var length = fromLen < toLen ? fromLen : toLen;
      var lastCommonSep = -1, i = 0;
      for (; i <= length; ++i) {
        if (i === length) {
          if (toLen > length) {
            if (to.charCodeAt(toStart + i) === 92) return toOrig.slice(toStart + i + 1);
            if (i === 2) return toOrig.slice(toStart + i);
          }
          if (fromLen > length) {
            if (from.charCodeAt(fromStart + i) === 92) lastCommonSep = i;
            else if (i === 2) lastCommonSep = 3;
          }
          break;
        }
        var fromCode = from.charCodeAt(fromStart + i);
        var toCode = to.charCodeAt(toStart + i);
        if (fromCode !== toCode) break;
        else if (fromCode === 92) lastCommonSep = i;
      }
      if (i !== length && lastCommonSep === -1) return toOrig;
      var out = "";
      if (lastCommonSep === -1) lastCommonSep = 0;
      for (i = fromStart + lastCommonSep + 1; i <= fromEnd; ++i) {
        if (i === fromEnd || from.charCodeAt(i) === 92) {
          if (out.length === 0) out += "..";
          else out += "\\..";
        }
      }
      if (out.length > 0) return out + toOrig.slice(toStart + lastCommonSep, toEnd);
      toStart += lastCommonSep;
      if (toOrig.charCodeAt(toStart) === 92) ++toStart;
      return toOrig.slice(toStart, toEnd);
    },
    dirname: function (path) {
      assertString(path, "path");
      var len = path.length;
      if (len === 0) return ".";
      var rootEnd = -1, end = -1, matchedSlash = true, offset = 0;
      var code = path.charCodeAt(0);
      if (len > 1) {
        if (isPathSeparator(code)) {
          rootEnd = offset = 1;
          if (isPathSeparator(path.charCodeAt(1))) {
            var j = 2, last = j;
            for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
            if (j < len && j !== last) {
              last = j;
              for (; j < len; ++j) if (!isPathSeparator(path.charCodeAt(j))) break;
              if (j < len && j !== last) {
                last = j;
                for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
                if (j === len) return path;
                if (j !== last) rootEnd = offset = j + 1;
              }
            }
          }
        } else if (isWindowsDeviceRoot(code) && path.charCodeAt(1) === 58) {
          rootEnd = len > 2 && isPathSeparator(path.charCodeAt(2)) ? 3 : 2; offset = rootEnd;
        }
      } else if (isPathSeparator(code)) return path;
      for (var i = len - 1; i >= offset; --i) {
        if (isPathSeparator(path.charCodeAt(i))) { if (!matchedSlash) { end = i; break; } }
        else matchedSlash = false;
      }
      if (end === -1) {
        if (rootEnd === -1) return ".";
        end = rootEnd;
      }
      return path.slice(0, end);
    },
    basename: function (path, ext) {
      if (ext !== undefined && typeof ext !== "string") throw new TypeError('"ext" must be a string');
      assertString(path, "path");
      var start = 0, end = -1, matchedSlash = true, i;
      if (path.length >= 2) {
        var drive = path.charCodeAt(0);
        if (isWindowsDeviceRoot(drive) && path.charCodeAt(1) === 58) start = 2;
      }
      if (ext !== undefined && ext.length > 0 && ext.length <= path.length) {
        if (ext.length === path.length && ext === path) return "";
        var extIdx = ext.length - 1, firstNonSlashEnd = -1;
        for (i = path.length - 1; i >= start; --i) {
          var code = path.charCodeAt(i);
          if (isPathSeparator(code)) { if (!matchedSlash) { start = i + 1; break; } }
          else {
            if (firstNonSlashEnd === -1) { matchedSlash = false; firstNonSlashEnd = i + 1; }
            if (extIdx >= 0) {
              if (code === ext.charCodeAt(extIdx)) { if (--extIdx === -1) end = i; }
              else { extIdx = -1; end = firstNonSlashEnd; }
            }
          }
        }
        if (start === end) end = firstNonSlashEnd;
        else if (end === -1) end = path.length;
        return path.slice(start, end);
      }
      for (i = path.length - 1; i >= start; --i) {
        if (isPathSeparator(path.charCodeAt(i))) { if (!matchedSlash) { start = i + 1; break; } }
        else if (end === -1) { matchedSlash = false; end = i + 1; }
      }
      return end === -1 ? "" : path.slice(start, end);
    },
    extname: function (path) {
      assertString(path, "path");
      var start = 0, startDot = -1, startPart = 0, end = -1, matchedSlash = true, preDotState = 0;
      if (path.length >= 2 && path.charCodeAt(1) === 58 && isWindowsDeviceRoot(path.charCodeAt(0))) start = startPart = 2;
      for (var i = path.length - 1; i >= start; --i) {
        var code = path.charCodeAt(i);
        if (isPathSeparator(code)) { if (!matchedSlash) { startPart = i + 1; break; } continue; }
        if (end === -1) { matchedSlash = false; end = i + 1; }
        if (code === 46) { if (startDot === -1) startDot = i; else if (preDotState !== 1) preDotState = 1; }
        else if (startDot !== -1) preDotState = -1;
      }
      if (startDot === -1 || end === -1 || preDotState === 0 ||
          (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)) return "";
      return path.slice(startDot, end);
    },
    format: function (pathObject) { return _format("\\", pathObject); },
    parse: function (path) {
      assertString(path, "path");
      var ret = { root: "", dir: "", base: "", ext: "", name: "" };
      if (path.length === 0) return ret;
      var len = path.length, rootEnd = 0, code = path.charCodeAt(0);
      if (len > 1) {
        if (isPathSeparator(code)) {
          rootEnd = 1;
          if (isPathSeparator(path.charCodeAt(1))) {
            var j = 2, last = j;
            for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
            if (j < len && j !== last) {
              last = j;
              for (; j < len; ++j) if (!isPathSeparator(path.charCodeAt(j))) break;
              if (j < len && j !== last) {
                last = j;
                for (; j < len; ++j) if (isPathSeparator(path.charCodeAt(j))) break;
                if (j === len) rootEnd = j;
                else if (j !== last) rootEnd = j + 1;
              }
            }
          }
        } else if (isWindowsDeviceRoot(code) && path.charCodeAt(1) === 58) {
          rootEnd = 2;
          if (len > 2 && isPathSeparator(path.charCodeAt(2))) {
            if (len === 3) { ret.root = ret.dir = path; return ret; }
            rootEnd = 3;
          }
        }
      } else if (isPathSeparator(code)) { ret.root = ret.dir = path; return ret; }
      if (rootEnd > 0) ret.root = path.slice(0, rootEnd);
      var startDot = -1, startPart = rootEnd, end = -1, matchedSlash = true, i = path.length - 1, preDotState = 0;
      for (; i >= rootEnd; --i) {
        code = path.charCodeAt(i);
        if (isPathSeparator(code)) { if (!matchedSlash) { startPart = i + 1; break; } continue; }
        if (end === -1) { matchedSlash = false; end = i + 1; }
        if (code === 46) { if (startDot === -1) startDot = i; else if (preDotState !== 1) preDotState = 1; }
        else if (startDot !== -1) preDotState = -1;
      }
      if (startDot === -1 || end === -1 || preDotState === 0 ||
          (preDotState === 1 && startDot === end - 1 && startDot === startPart + 1)) {
        if (end !== -1) ret.base = ret.name = path.slice(startPart, end);
      } else {
        ret.name = path.slice(startPart, startDot);
        ret.base = path.slice(startPart, end);
        ret.ext = path.slice(startDot, end);
      }
      if (startPart > 0 && startPart !== rootEnd) ret.dir = path.slice(0, startPart - 1);
      else ret.dir = ret.root;
      return ret;
    },
    toNamespacedPath: function (path) {
      if (typeof path !== "string") return path;
      if (path.length === 0) return "";
      var resolvedPath = win32.resolve(path);
      if (resolvedPath.length >= 3) {
        if (resolvedPath.charCodeAt(0) === 92) {
          if (resolvedPath.charCodeAt(1) === 92) {
            var code = resolvedPath.charCodeAt(2);
            if (code !== 63 && code !== 46) return "\\\\?\\UNC\\" + resolvedPath.slice(2);
          }
        } else if (isWindowsDeviceRoot(resolvedPath.charCodeAt(0))
          && resolvedPath.charCodeAt(1) === 58 && resolvedPath.charCodeAt(2) === 92) {
          return "\\\\?\\" + resolvedPath;
        }
      }
      return path;
    },
  };
  win32.win32 = win32;

  function _format(sep, pathObject) {
    if (pathObject === null || typeof pathObject !== "object") {
      throw new TypeError('"pathObject" must be a non-null object');
    }
    var dir = pathObject.dir || pathObject.root;
    var base = pathObject.base || ((pathObject.name || "") + (pathObject.ext || ""));
    if (!dir) return base;
    return dir === pathObject.root ? dir + base : dir + sep + base;
  }

  posix.win32 = win32;
  win32.posix = posix;

  // Pick the platform-default — same logic Node uses.
  var p = (globalThis.__tynd_platform__ && globalThis.__tynd_platform__()) || "linux";
  var path = p === "win32" ? win32 : posix;

  globalThis.__tynd_modules = globalThis.__tynd_modules || Object.create(null);
  globalThis.__tynd_modules["path"] = path;
  globalThis.__tynd_modules["node:path"] = path;
  globalThis.__tynd_modules["path/posix"] = posix;
  globalThis.__tynd_modules["node:path/posix"] = posix;
  globalThis.__tynd_modules["path/win32"] = win32;
  globalThis.__tynd_modules["node:path/win32"] = win32;
})();
