#!/usr/bin/env bun
// Thorough internal link checker. Scans every MDX file for links to
// /docs/v0.1/... (markdown + JSX href) and verifies each target resolves
// to an actual page or anchor.

import { readdir, stat } from "node:fs/promises";
import { join, relative } from "node:path";

const CONTENT = "src/content/v0.1";

async function walk(dir: string): Promise<string[]> {
  const out: string[] = [];
  for (const entry of await readdir(dir)) {
    const p = join(dir, entry);
    const s = await stat(p);
    if (s.isDirectory()) out.push(...(await walk(p)));
    else if (entry.endsWith(".mdx")) out.push(p);
  }
  return out;
}

function routePathForFile(filePath: string): string {
  const rel = relative(CONTENT, filePath).replace(/\\/g, "/");
  const stripped = rel.replace(/\.mdx$/, "");
  const noIndex = stripped.endsWith("/index") ? stripped.slice(0, -"/index".length) : stripped;
  return `/docs/v0.1/${noIndex}`.replace(/\/+$/, "") || "/docs/v0.1";
}

async function main() {
  const files = await walk(CONTENT);
  const routes = new Set<string>();
  for (const f of files) {
    routes.add(routePathForFile(f));
  }
  // `/docs/v0.1` root shorthand
  routes.add("/docs/v0.1");

  const linkPattern = /\]\((\/docs\/v0\.1[^)#?\s]*)[^)]*\)|href="(\/docs\/v0\.1[^"#?]*)"/g;
  const broken: Array<{ from: string; link: string }> = [];
  let total = 0;

  for (const f of files) {
    const src = await Bun.file(f).text();
    let m: RegExpExecArray | null;
    linkPattern.lastIndex = 0;
    // biome-ignore lint/suspicious/noAssignInExpressions: regex loop
    while ((m = linkPattern.exec(src)) !== null) {
      const link = (m[1] ?? m[2] ?? "").replace(/\/$/, "");
      if (!link) continue;
      total++;
      if (!routes.has(link)) broken.push({ from: f, link });
    }
  }

  const unique = new Set(broken.map((b) => b.link));

  console.log(`scanned: ${files.length} files, ${total} internal links`);
  console.log(`unique targets present: ${routes.size}`);
  console.log(`broken: ${broken.length} (${unique.size} unique)`);

  if (broken.length) {
    console.log("\nBROKEN:");
    const seen = new Set<string>();
    for (const b of broken) {
      const key = `${b.from} → ${b.link}`;
      if (seen.has(key)) continue;
      seen.add(key);
      console.log(`  ${b.from} → ${b.link}`);
    }
  } else {
    console.log("\nAll internal /docs/v0.1/* links resolve.");
  }
}

void main();
