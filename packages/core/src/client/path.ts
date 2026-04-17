export const path = {
  sep(): "/" | "\\" {
    return typeof navigator !== "undefined" && /win/i.test(navigator.platform) ? "\\" : "/"
  },
  join(...parts: string[]): string {
    const s = path.sep()
    return parts
      .filter((p) => p.length > 0)
      .join(s)
      .replace(/[/\\]+/g, s)
  },
  dirname(p: string): string {
    const m = p.replace(/[/\\]$/, "").match(/^(.*)[/\\][^/\\]+$/)
    return m ? m[1]! : ""
  },
  basename(p: string, ext?: string): string {
    const name =
      p
        .replace(/[/\\]$/, "")
        .split(/[/\\]/)
        .pop() ?? ""
    return ext && name.endsWith(ext) ? name.slice(0, -ext.length) : name
  },
  extname(p: string): string {
    const name = path.basename(p)
    const i = name.lastIndexOf(".")
    return i > 0 ? name.slice(i) : ""
  },
}
