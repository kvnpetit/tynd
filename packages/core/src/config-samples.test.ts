import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import * as v from "valibot";
import { AppConfigSchema, configIssues } from "./types";

// Every `app.start({ … })` sample shipped in the docs, the CLI scaffold and the
// plugin READMEs must use only known option keys at every nesting level, so the
// strict schema never rejects a config a user copied from us.

const ROOT = fileURLToPath(new URL("../../../", import.meta.url));

function walk(dir: string, keep: (path: string) => boolean, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === "dist" || name === "target") continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, keep, out);
    else if (keep(path)) out.push(path);
  }
  return out;
}

function sampleSources(): { file: string; code: string }[] {
  const sources: { file: string; code: string }[] = [];
  const fenced = (file: string) => {
    for (const m of readFileSync(file, "utf8").matchAll(/^\s*```[^\n]*\n([\s\S]*?)^\s*```/gm)) {
      sources.push({ file, code: m[1] ?? "" });
    }
  };
  for (const file of walk(join(ROOT, "apps/docs/src/content"), (p) => /\.mdx?$/.test(p))) {
    fenced(file);
  }
  for (const group of readdirSync(join(ROOT, "plugins"))) {
    const groupDir = join(ROOT, "plugins", group);
    if (!statSync(groupDir).isDirectory()) continue;
    for (const plugin of readdirSync(groupDir)) {
      const readme = join(groupDir, plugin, "README.md");
      try {
        fenced(readme);
      } catch {
        // no README
      }
    }
  }
  const templates = join(ROOT, "crates/cli/templates");
  for (const file of walk(templates, (p) => /\.(ts|tsx|js|md)$/.test(p))) {
    sources.push({ file, code: readFileSync(file, "utf8") });
  }
  for (const file of walk(join(ROOT, "crates/cli/src"), (p) => /scaffold[^/\\]*\.rs$/.test(p))) {
    // Rust `format!` templates: unescape braces, stub interpolations.
    const code = readFileSync(file, "utf8").replace(/\{\{|\}\}|\{[a-z_]+\}/g, (m) =>
      m === "{{" ? "{" : m === "}}" ? "}" : "x",
    );
    sources.push({ file, code });
  }
  return sources;
}

/** Index just past the string literal that opens at `start`. */
function skipString(code: string, start: number): number {
  const quote = code[start];
  let i = start + 1;
  while (i < code.length && code[i] !== quote) i += code[i] === "\\" ? 2 : 1;
  return i + 1;
}

/** Index just past the bracket that closes the one at `start`. */
function skipBalanced(code: string, start: number): number {
  let depth = 0;
  let i = start;
  while (i < code.length) {
    const c = code[i] ?? "";
    if ("\"'`".includes(c)) {
      i = skipString(code, i);
      continue;
    }
    if ("{[(".includes(c)) depth++;
    else if ("}])".includes(c) && --depth === 0) return i + 1;
    i++;
  }
  return i;
}

/** The argument text of every `app.start({ … })` call in `code`. */
function startArgs(code: string): string[] {
  return [...code.matchAll(/app\.start\(\s*\{/g)].map((m) => {
    const begin = (m.index ?? 0) + m[0].length - 1;
    return code.slice(begin, skipBalanced(code, begin));
  });
}

// Any free identifier in a sample (`appName`, `process.env.X`, `path.join(…)`)
// resolves to this inert stand-in: values may fail validation, keys may not.
const stub: unknown = new Proxy(() => stub, {
  get: (_t, key) =>
    key === Symbol.toPrimitive ? () => "x" : key === Symbol.iterator ? [][Symbol.iterator] : stub,
  ownKeys: () => [],
});
const scope = new Proxy(
  {},
  { has: () => true, get: (_t, key) => (key === Symbol.unscopables ? undefined : stub) },
);
const transpiler = new Bun.Transpiler({ loader: "ts" });

function evaluate(arg: string): unknown {
  const js = transpiler
    .transformSync(`export default (${arg.replace(/import\.meta/g, "__meta")});`)
    .replace(/export default/, "return");
  return new Function("scope", `with (scope) { ${js} }`)(scope);
}

describe("app.start samples", () => {
  const samples = sampleSources().flatMap(({ file, code }) =>
    startArgs(code).map((arg) => ({ file: relative(ROOT, file).replaceAll("\\", "/"), arg })),
  );

  test("are found in the docs, the scaffold and the plugin READMEs", () => {
    expect(samples.length).toBeGreaterThan(20);
    expect(samples.some((s) => s.file.includes("scaffold"))).toBe(true);
    expect(samples.some((s) => s.file.startsWith("plugins/"))).toBe(true);
  });

  test("the checker flags a nested typo next to free identifiers", () => {
    expect(unknownKeys("x.md", "{ window: { titel: appName, width: size.w } } as const")).toEqual([
      'x.md: window.titel: unknown option "titel"; valid options: title, width, height, minWidth, minHeight, maxWidth, maxHeight, resizable, decorations, transparent, alwaysOnTop, center, fullscreen, maximized, visible, kiosk, titleBarStyle, x, y, theme, skipTaskbar, backgroundColor, userAgent, incognito, newWindow, downloads, proxy, permissions, webgpu, hardwareAcceleration, preferredFps, contextLostRecovery, colorSpace, hdr, vrr, gpu',
    ]);
  });

  test("use only known option keys", () => {
    expect(samples.flatMap(({ file, arg }) => unknownKeys(file, arg))).toEqual([]);
  });
});

function unknownKeys(file: string, arg: string): string[] {
  let config: unknown;
  try {
    config = evaluate(arg);
  } catch (e) {
    return [`${file}: cannot evaluate sample (${(e as Error).message}):\n${arg}`];
  }
  return configIssues(v.safeParse(AppConfigSchema, config).issues ?? [])
    .filter((issue) => issue.unknownKey)
    .map((issue) => `${file}: ${issue.path}: ${issue.message}`);
}
