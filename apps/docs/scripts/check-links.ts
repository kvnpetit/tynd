// Post-build link checker: every internal href/src in dist/**/*.html must resolve to a
// file in dist (honoring the site base). Run after `bun run build`.
import { readdir, readFile, stat } from "node:fs/promises";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { BASE, SITE_ORIGIN } from "../src/lib/site";

const DIST = join(dirname(fileURLToPath(import.meta.url)), "..", "dist");

async function walk(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...(await walk(path)));
    else if (entry.name.endsWith(".html")) out.push(path);
  }
  return out;
}

const ATTR = /\s(?:href|src)="([^"]+)"/g;
const ids = new Map<string, Set<string>>();

async function pageIds(file: string): Promise<Set<string>> {
  let set = ids.get(file);
  if (!set) {
    const html = await readFile(file, "utf8");
    set = new Set([...html.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1] ?? ""));
    ids.set(file, set);
  }
  return set;
}

/** The dist file a base-prefixed URL path serves (`<path>` or `<path>/index.html`). */
async function resolveFile(path: string): Promise<string | undefined> {
  const rel = decodeURIComponent(path.slice(BASE.length)).replace(/^\//, "");
  for (const candidate of [join(DIST, rel), join(DIST, rel, "index.html")]) {
    const info = await stat(candidate).catch(() => undefined);
    if (info?.isFile()) return candidate;
  }
  return undefined;
}

const broken: string[] = [];
let checked = 0;

for (const file of await walk(DIST)) {
  if (file.includes(`${sep}_pagefind${sep}`)) continue;
  const html = await readFile(file, "utf8");
  const page = `/${relative(DIST, file).split(sep).join("/")}`;
  for (const match of html.matchAll(ATTR)) {
    let url = match[1] ?? "";
    if (url.startsWith(SITE_ORIGIN)) url = url.slice(SITE_ORIGIN.length);
    if (!url.startsWith("/") || url.startsWith("//")) continue; // external, mailto:, #hash...
    const [pathWithQuery = "", hash] = url.split("#");
    const path = pathWithQuery.split("?")[0] ?? "";
    checked++;
    if (path !== BASE && !path.startsWith(`${BASE}/`)) {
      broken.push(`${page}: ${url} (missing base ${BASE})`);
      continue;
    }
    const target = await resolveFile(path);
    if (!target) {
      broken.push(`${page}: ${url}`);
    } else if (hash && target.endsWith(".html") && !(await pageIds(target)).has(hash)) {
      broken.push(`${page}: ${url} (missing anchor)`);
    }
  }
}

console.log(`[check-links] checked ${checked} internal links`);
if (broken.length > 0) {
  const unique = [...new Set(broken)];
  console.error(`[check-links] ${unique.length} broken link(s):`);
  for (const line of unique.slice(0, 200)) console.error(`  ${line}`);
  process.exit(1);
}
