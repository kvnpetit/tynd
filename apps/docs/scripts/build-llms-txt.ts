// Generates llms.txt + llms-full.txt for the docs site.
// Output layout (mirrors the /docs route tree):
//   public/docs/<version>/llms.txt        ← index
//   public/docs/<version>/llms-full.txt   ← full content
//   public/docs/llms{,-full}.txt          ← same files (mirrors the /docs redirect)
// Plus public/llms.txt + public/llms-full.txt at the site root so crawlers
// that follow the llmstxt.org spec find the manifest without knowing the
// docs namespace.

import { mkdir, readdir, stat } from "node:fs/promises";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { SITE } from "../src/lib/site";
import { LATEST_SLUG } from "../src/lib/versions";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROJECT = join(__dirname, "..");
const DOCS = join(PROJECT, "src", "content", "docs", "docs");
const PUBLIC = join(PROJECT, "public");

const SITE_URL = SITE.url;
const SITE_NAME = SITE.name;
const SITE_TAGLINE = SITE.description;

const CONTENT = join(DOCS, LATEST_SLUG);

interface Page {
  route: string;
  section: string;
  title: string;
  description: string;
  body: string;
}

async function walk(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir)) {
    if (entry.startsWith("_") || entry === ".gitkeep") continue;
    const p = join(dir, entry);
    const info = await stat(p);
    if (info.isDirectory()) out.push(...(await walk(p)));
    else if (entry.endsWith(".mdx")) out.push(p);
  }
  return out;
}

function parseFrontmatter(src: string): {
  fm: Record<string, string>;
  body: string;
} {
  const m = src.match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!m) return { fm: {}, body: src };
  const [, rawFm = "", body = ""] = m;
  const fm: Record<string, string> = {};
  for (const line of rawFm.split("\n")) {
    const idx = line.indexOf(":");
    if (idx === -1) continue;
    const key = line.slice(0, idx).trim();
    let val = line.slice(idx + 1).trim();
    if (val.startsWith('"') && val.endsWith('"')) val = val.slice(1, -1);
    fm[key] = val;
  }
  return { fm, body };
}

function routeForFile(filePath: string): {
  route: string;
  section: string;
} {
  const rel = relative(DOCS, filePath).split(sep).join("/");
  const withoutExt = rel.replace(/\.mdx$/, "");
  const stripped = withoutExt.endsWith("/index")
    ? withoutExt.slice(0, -"/index".length)
    : withoutExt;
  const segments = stripped.split("/");
  const section = segments[1] ?? "root";
  return {
    route: `/docs/${stripped}`.replace(/\/+$/, "") || "/docs",
    section,
  };
}

function pageSortKey(p: Page): string {
  return p.route.replace(/\//g, "\u0001");
}

/** Page bodies use root-relative links; LLM consumers need absolute URLs (incl. the site base). */
function absolutizeLinks(body: string): string {
  return body
    .replace(
      /^import .* from ["'](?:@astrojs\/starlight\/components|@\/components\/[^"']+)["'];\n+/gm,
      "",
    )
    .replace(/\]\((\/docs\/[^)\s]*)\)/g, (_m, path: string) => `](${SITE_URL}${path})`)
    .replace(/href="(\/docs\/[^"]*)"/g, (_m, path: string) => `href="${SITE_URL}${path}"`);
}

async function loadPages(): Promise<Page[]> {
  const files = await walk(CONTENT);
  const pages: Page[] = [];
  for (const filePath of files) {
    const src = await Bun.file(filePath).text();
    const { fm, body } = parseFrontmatter(src);
    const { route, section } = routeForFile(filePath);
    pages.push({
      route,
      section,
      title: fm.title ?? route,
      description: fm.description ?? "",
      body: absolutizeLinks(body.trim()),
    });
  }
  pages.sort((a, b) => pageSortKey(a).localeCompare(pageSortKey(b)));
  return pages;
}

function groupBySection(pages: Page[]): Map<string, Page[]> {
  const groups = new Map<string, Page[]>();
  for (const p of pages) {
    const list = groups.get(p.section) ?? [];
    list.push(p);
    groups.set(p.section, list);
  }
  return groups;
}

const SECTION_ORDER = [
  "root",
  "getting-started",
  "concepts",
  "guides",
  "tutorials",
  "recipes",
  "api",
  "cli",
  "runtimes",
  "compare",
  "production-checklist",
  "faq",
  "glossary",
  "troubleshooting",
  "contributing",
];

const SECTION_TITLES: Record<string, string> = {
  root: "Overview",
  "getting-started": "Getting Started",
  concepts: "Core Concepts",
  guides: "Guides",
  tutorials: "Tutorials",
  recipes: "Recipes",
  api: "API Reference",
  cli: "CLI Reference",
  runtimes: "Runtimes",
  compare: "Comparisons",
  "production-checklist": "Production Checklist",
  faq: "FAQ",
  glossary: "Glossary",
  troubleshooting: "Troubleshooting",
  contributing: "Contributing",
};

const sectionRank = (section: string) => {
  const rank = SECTION_ORDER.indexOf(section);
  return rank === -1 ? 999 : rank;
};

/** Pages grouped by section, sections in `SECTION_ORDER` (unknown ones last). */
function sortedSections(pages: Page[]): [string, Page[]][] {
  return [...groupBySection(pages)].sort(([a], [b]) => sectionRank(a) - sectionRank(b));
}

function renderIndex(pages: Page[], version: string): string {
  const lines: string[] = [];

  lines.push(`# ${SITE_NAME} — ${version}`, "");
  lines.push(`> ${SITE_TAGLINE}`, "");
  lines.push(
    `${SITE_NAME} is a desktop-app framework with a TypeScript backend running on Bun and a native WebView front-end. Every app runs on a native Rust host that the CLI compiles with Cargo from \`@tynd/host-src\`, containing the core plus the APIs of the declared plugins. IPC is zero-network (no TCP), RPC is zero-codegen (types flow from \`typeof backend\`), and OS capabilities are installed explicitly as plugins.`,
    "",
  );

  for (const [section, sectionPages] of sortedSections(pages)) {
    lines.push(`## ${SECTION_TITLES[section] ?? section}`, "");
    for (const p of sectionPages) {
      const url = `${SITE_URL}${p.route}`;
      const desc = p.description ? `: ${p.description}` : "";
      lines.push(`- [${p.title}](${url})${desc}`);
    }
    lines.push("");
  }

  lines.push("## Resources", "");
  lines.push(`- [GitHub repository](${SITE.links.github})`);
  lines.push(`- [npm: @tynd/cli](${SITE.links.npm})`);
  lines.push(`- [npm: @tynd/core](https://www.npmjs.com/package/@tynd/core)`);
  lines.push(
    `- [llms-full.txt (${version})](${SITE_URL}/docs/${version}/llms-full.txt) — full content of every page in this version`,
  );
  lines.push("");

  return lines.join("\n");
}

function renderFull(pages: Page[], version: string): string {
  const out: string[] = [];

  out.push(`# ${SITE_NAME} — ${version}`, "");
  out.push(`> ${SITE_TAGLINE}`, "");
  out.push(
    `Complete documentation for ${SITE_NAME} ${version}. Each page below includes its URL, title, description, and full body content. Intended for LLM ingestion.`,
    "",
  );

  for (const [section, sectionPages] of sortedSections(pages)) {
    out.push(`--- SECTION: ${SECTION_TITLES[section] ?? section} ---`, "");
    for (const p of sectionPages) {
      const url = `${SITE_URL}${p.route}`;
      out.push(`URL: ${url}`);
      out.push(`TITLE: ${p.title}`);
      if (p.description) out.push(`DESCRIPTION: ${p.description}`);
      out.push("");
      out.push(p.body);
      out.push("", "----", "");
    }
  }

  return out.join("\n");
}

async function main() {
  const pages = await loadPages();
  const index = renderIndex(pages, LATEST_SLUG);
  const full = renderFull(pages, LATEST_SLUG);

  // /docs/<version>/, /docs/ (mirrors the /docs redirect) and the site root (llmstxt.org).
  const docsDir = join(PUBLIC, "docs");
  for (const dir of [join(docsDir, LATEST_SLUG), docsDir, PUBLIC]) {
    await mkdir(dir, { recursive: true });
    await Bun.write(join(dir, "llms.txt"), index);
    await Bun.write(join(dir, "llms-full.txt"), full);
  }
  console.log(
    `[llms-txt] wrote ${pages.length} pages (${LATEST_SLUG}) → public/{,docs/,docs/${LATEST_SLUG}/}llms{,-full}.txt`,
  );
}

void main();
