#!/usr/bin/env bun
// Generates per-version llms.txt + llms-full.txt for the docs site.
//
// Output layout (mirrors the /docs route tree):
//   public/docs/llms.txt               ← latest version index
//   public/docs/llms-full.txt          ← latest version full content
//   public/docs/v0.1/llms.txt          ← v0.1 index
//   public/docs/v0.1/llms-full.txt     ← v0.1 full content
//   public/docs/v0.2/llms.txt          ← (when v0.2 ships)
//   …
// Plus public/llms.txt + public/llms-full.txt at the site root so crawlers
// that follow the llmstxt.org spec find the manifest without knowing the
// docs namespace.

import { mkdir, readdir, stat } from "node:fs/promises";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const PROJECT = join(__dirname, "..");
const CONTENT = join(PROJECT, "src", "content");
const PUBLIC = join(PROJECT, "public");

const SITE_URL = process.env.NEXT_PUBLIC_BASE_URL ?? "https://tynd.dev";
const SITE_NAME = "Tynd";
const SITE_TAGLINE =
  "Desktop apps in TypeScript. Small native binaries, zero-codegen typed RPC.";

/** Latest version slug — must match lib/versions.ts. */
const LATEST_VERSION = "v0.1";

interface Page {
  filePath: string;
  route: string;
  versionSlug: string;
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

function parseFrontmatter(src: string): { fm: Record<string, string>; body: string } {
  const m = src.match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!m) return { fm: {}, body: src };
  const fm: Record<string, string> = {};
  for (const line of m[1].split("\n")) {
    const idx = line.indexOf(":");
    if (idx === -1) continue;
    const key = line.slice(0, idx).trim();
    let val = line.slice(idx + 1).trim();
    if (val.startsWith('"') && val.endsWith('"')) val = val.slice(1, -1);
    fm[key] = val;
  }
  return { fm, body: m[2] };
}

function routeForFile(filePath: string): {
  route: string;
  versionSlug: string;
  section: string;
} {
  const rel = relative(CONTENT, filePath).split(sep).join("/");
  const withoutExt = rel.replace(/\.mdx$/, "");
  const stripped = withoutExt.endsWith("/index")
    ? withoutExt.slice(0, -"/index".length)
    : withoutExt;
  const segments = stripped.split("/");
  const versionSlug = segments[0] ?? "";
  const section = segments[1] ?? "root";
  return {
    route: `/docs/${stripped}`.replace(/\/+$/, "") || "/docs",
    versionSlug,
    section,
  };
}

function pageSortKey(p: Page): string {
  return p.route.replace(/\//g, "\u0001");
}

async function loadPages(): Promise<Page[]> {
  const files = await walk(CONTENT);
  const pages: Page[] = [];
  for (const filePath of files) {
    const src = await Bun.file(filePath).text();
    const { fm, body } = parseFrontmatter(src);
    const { route, versionSlug, section } = routeForFile(filePath);
    pages.push({
      filePath,
      route,
      versionSlug,
      section,
      title: fm.title ?? route,
      description: fm.description ?? "",
      body: body.trim(),
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

function renderIndex(pages: Page[], version: string): string {
  const groups = groupBySection(pages);
  const lines: string[] = [];

  lines.push(`# ${SITE_NAME} — ${version}`, "");
  lines.push(`> ${SITE_TAGLINE}`, "");
  lines.push(
    `${SITE_NAME} is a desktop-app framework with a TypeScript backend, a native WebView front-end, and small native binaries. It ships two runtimes — \`lite\` (~6.5 MB, embedded JS engine) and \`full\` (~44 MB, Bun subprocess) — from the same TypeScript source. IPC is zero-network (no TCP), RPC is zero-codegen (types flow from \`typeof backend\`), and 26 OS APIs behave identically across both runtimes.`,
    "",
  );

  const sortedSections = [...groups.keys()].sort(
    (a, b) =>
      (SECTION_ORDER.indexOf(a) === -1 ? 999 : SECTION_ORDER.indexOf(a)) -
      (SECTION_ORDER.indexOf(b) === -1 ? 999 : SECTION_ORDER.indexOf(b)),
  );

  for (const section of sortedSections) {
    const sectionPages = groups.get(section);
    if (!sectionPages?.length) continue;
    lines.push(`## ${SECTION_TITLES[section] ?? section}`, "");
    for (const p of sectionPages) {
      const url = `${SITE_URL}${p.route}`;
      const desc = p.description ? `: ${p.description}` : "";
      lines.push(`- [${p.title}](${url})${desc}`);
    }
    lines.push("");
  }

  lines.push("## Resources", "");
  lines.push(`- [GitHub repository](https://github.com/kvnpetit/tynd)`);
  lines.push(`- [npm: @tynd/cli](https://www.npmjs.com/package/@tynd/cli)`);
  lines.push(`- [npm: @tynd/core](https://www.npmjs.com/package/@tynd/core)`);
  lines.push(
    `- [llms-full.txt (${version})](${SITE_URL}/docs/${version}/llms-full.txt) — full content of every page in this version`,
  );
  lines.push("");

  return lines.join("\n");
}

function renderFull(pages: Page[], version: string): string {
  const groups = groupBySection(pages);
  const out: string[] = [];

  out.push(`# ${SITE_NAME} — ${version}`, "");
  out.push(`> ${SITE_TAGLINE}`, "");
  out.push(
    `Complete documentation for ${SITE_NAME} ${version}. Each page below includes its URL, title, description, and full body content. Intended for LLM ingestion.`,
    "",
  );

  const sortedSections = [...groups.keys()].sort(
    (a, b) =>
      (SECTION_ORDER.indexOf(a) === -1 ? 999 : SECTION_ORDER.indexOf(a)) -
      (SECTION_ORDER.indexOf(b) === -1 ? 999 : SECTION_ORDER.indexOf(b)),
  );

  for (const section of sortedSections) {
    const sectionPages = groups.get(section);
    if (!sectionPages?.length) continue;
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

  const byVersion = new Map<string, Page[]>();
  for (const p of pages) {
    if (!p.versionSlug || !p.versionSlug.startsWith("v")) continue;
    const list = byVersion.get(p.versionSlug) ?? [];
    list.push(p);
    byVersion.set(p.versionSlug, list);
  }

  const versions = [...byVersion.keys()].sort();

  const DOCS_DIR = join(PUBLIC, "docs");
  await mkdir(DOCS_DIR, { recursive: true });

  for (const version of versions) {
    const versionPages = byVersion.get(version) ?? [];
    const dir = join(DOCS_DIR, version);
    await mkdir(dir, { recursive: true });

    const index = renderIndex(versionPages, version);
    const full = renderFull(versionPages, version);
    await Bun.write(join(dir, "llms.txt"), index);
    await Bun.write(join(dir, "llms-full.txt"), full);

    console.log(
      `[llms-txt] wrote ${versionPages.length} pages → public/docs/${version}/llms.txt + llms-full.txt`,
    );
  }

  const latestPages = byVersion.get(LATEST_VERSION) ?? [];
  if (latestPages.length) {
    const index = renderIndex(latestPages, LATEST_VERSION);
    const full = renderFull(latestPages, LATEST_VERSION);
    // /docs/llms.txt → latest version (mirrors /docs route-level redirect).
    await Bun.write(join(DOCS_DIR, "llms.txt"), index);
    await Bun.write(join(DOCS_DIR, "llms-full.txt"), full);
    // /llms.txt → site-root pointer per llmstxt.org spec so crawlers find
    // the manifest without knowing the /docs namespace.
    await Bun.write(join(PUBLIC, "llms.txt"), index);
    await Bun.write(join(PUBLIC, "llms-full.txt"), full);
    console.log(
      `[llms-txt] mirrored ${LATEST_VERSION} → public/docs/llms{,-full}.txt + public/llms{,-full}.txt`,
    );
  }
}

void main();
