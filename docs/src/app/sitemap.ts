import { stat } from "node:fs/promises";
import path from "node:path";
import type { MetadataRoute } from "next";
import { getPageMap } from "nextra/page-map";
import { SITE } from "../lib/site";
import { LATEST_SLUG } from "../lib/versions";

export const dynamic = "force-static";

type PageNode = {
  route?: string;
  name?: string;
  children?: PageNode[];
};

type DocRoute = { route: string; filePath: string };

const CONTENT_ROOT = path.join(process.cwd(), "src", "content");

function collectDocRoutes(nodes: readonly PageNode[]): DocRoute[] {
  const out: DocRoute[] = [];
  for (const node of nodes) {
    if (node.route) {
      const rel = node.route.replace(/^\/docs\//, "");
      const filePath = path.join(CONTENT_ROOT, `${rel}.mdx`);
      out.push({ route: node.route, filePath });
    }
    if (node.children) out.push(...collectDocRoutes(node.children));
  }
  return out;
}

async function fileMtime(filePath: string): Promise<Date | undefined> {
  try {
    return (await stat(filePath)).mtime;
  } catch {
    return undefined;
  }
}

function withTrailingSlash(p: string): string {
  if (p === "/") return p;
  return p.endsWith("/") ? p : `${p}/`;
}

const STATIC_ROUTES: ReadonlyArray<{
  path: string;
  priority: number;
  changeFrequency: "daily" | "weekly" | "monthly";
}> = [
  { path: "/", priority: 1.0, changeFrequency: "weekly" },
  { path: `/docs/${LATEST_SLUG}/`, priority: 0.9, changeFrequency: "weekly" },
  { path: "/showcase/", priority: 0.8, changeFrequency: "weekly" },
  { path: "/blog/", priority: 0.7, changeFrequency: "weekly" },
  { path: "/changelog/", priority: 0.6, changeFrequency: "weekly" },
  { path: "/compare/", priority: 0.6, changeFrequency: "monthly" },
];

export default async function sitemap(): Promise<MetadataRoute.Sitemap> {
  const pageMap = (await getPageMap("/docs")) as PageNode[];
  const docRoutes = collectDocRoutes(pageMap);

  const now = new Date();
  const seen = new Set<string>();
  const entries: MetadataRoute.Sitemap = [];

  for (const r of STATIC_ROUTES) {
    const url = `${SITE.url}${r.path}`;
    if (seen.has(url)) continue;
    seen.add(url);
    entries.push({
      url,
      priority: r.priority,
      changeFrequency: r.changeFrequency,
      lastModified: now,
    });
  }

  for (const { route, filePath } of docRoutes) {
    const url = `${SITE.url}${withTrailingSlash(route)}`;
    if (seen.has(url)) continue;
    seen.add(url);
    const mtime = await fileMtime(filePath);
    const isLatest = route.includes(`/${LATEST_SLUG}`);
    entries.push({
      url,
      priority: isLatest ? 0.8 : 0.3,
      changeFrequency: isLatest ? "weekly" : "monthly",
      lastModified: mtime ?? now,
    });
  }

  return entries;
}
