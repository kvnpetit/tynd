// Static redirects for GitHub Pages (no server-side rules): Astro emits a
// meta-refresh page for each entry. Keys are root-relative paths (Astro serves them
// under `base`); destinations must include the base themselves.
import { readdirSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { BASE } from "./site";
import { LATEST_SLUG } from "./versions";

const LATEST_DIR = join(
  dirname(fileURLToPath(import.meta.url)),
  "..",
  "content",
  "docs",
  "docs",
  LATEST_SLUG,
);

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...walk(path));
    else if (entry.name.endsWith(".mdx")) out.push(path);
  }
  return out;
}

/** Page slugs of the latest version relative to it (`""` is the version index). */
function latestSlugs(): string[] {
  return walk(LATEST_DIR)
    .map((file) =>
      relative(LATEST_DIR, file)
        .split(sep)
        .join("/")
        .replace(/\.mdx$/, "")
        .replace(/(^|\/)index$/, ""),
    )
    .sort();
}

export function buildRedirects(): Record<string, string> {
  const latest = `/docs/${LATEST_SLUG}`;
  const target = `${BASE}${latest}`;
  const redirects: Record<string, string> = {
    "/docs": `${target}/`,
    "/docs/latest": `${target}/`,
    "/compare": `${target}/compare/`,
  };

  const slugs = latestSlugs();
  for (const slug of slugs) {
    if (!slug) continue;
    // `/docs/<slug>` and `/docs/latest/<slug>` -> latest version.
    redirects[`/docs/${slug}`] = `${target}/${slug}/`;
    redirects[`/docs/latest/${slug}`] = `${target}/${slug}/`;
  }

  // Catalog docs links use /docs/<latest>/plugins/<id>.
  for (const slug of slugs) {
    const match = slug.match(/^api\/plugins\/([^/]+)$/);
    if (match) redirects[`${latest}/plugins/${match[1]}`] = `${target}/${slug}/`;
  }

  return redirects;
}
