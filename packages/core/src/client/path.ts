/**
 * Frontend path helpers — auto-detect Windows vs POSIX from `navigator.platform`
 * and forward to the matching `pathe` backend. `pathe` ships pure-JS
 * implementations of both flavours (no `node:path` runtime dep) so the
 * helpers work inside the WebView.
 * Callers use `path.join`, `path.dirname`, etc. without knowing about pathe.
 *
 * Contract: every helper that builds a path outputs `/` as the separator,
 * Windows included (pathe normalizes `\` to `/`; Windows APIs accept it).
 * Inputs may use either separator. There is deliberately no `sep()`: a native
 * `\` would contradict what `join` produces.
 */

import { posix, win32 } from "pathe";

const _isWindows = typeof navigator !== "undefined" && /win/i.test(navigator.platform ?? "");
const _impl = _isWindows ? win32 : posix;

export const path = {
  join(...parts: string[]): string {
    // Filter empties to match the previous behaviour (pathe.join would
    // otherwise emit a `.` for `join("")`).
    const cleaned = parts.filter((p) => p.length > 0);
    return cleaned.length === 0 ? "" : _impl.join(...cleaned);
  },
  dirname(p: string): string {
    const stripped = p.replace(/[/\\]$/, "");
    if (!stripped.includes("/") && !stripped.includes("\\")) return "";
    const result = _impl.dirname(stripped);
    return result === "." ? "" : result;
  },
  basename(p: string, ext?: string): string {
    return _impl.basename(p.replace(/[/\\]$/, ""), ext);
  },
  extname(p: string): string {
    return _impl.extname(p);
  },
};
