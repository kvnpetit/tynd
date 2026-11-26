// Builds the Starlight sidebar from the per-directory `meta.json` page order.
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { LATEST_SLUG } from "./versions";

type SidebarItem =
  | { slug: string }
  | { label: string; link: string }
  | { label: string; items: SidebarItem[]; collapsed?: boolean };

type Meta = { title?: string; pages?: string[] };

const DOCS_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "content", "docs");
const SEPARATOR = /^---(.+)---$/;
const LINK = /^\[(.+)\]\((.+)\)$/;

function readMeta(dir: string): Meta | undefined {
  const file = join(DOCS_ROOT, dir, "meta.json");
  if (!existsSync(file)) return;
  return JSON.parse(readFileSync(file, "utf8")) as Meta;
}

function buildDir(dir: string): SidebarItem[] {
  const meta = readMeta(dir);
  const root: SidebarItem[] = [];
  let current = root;

  for (const name of meta?.pages ?? []) {
    const separator = SEPARATOR.exec(name);
    if (separator?.[1]) {
      const group: SidebarItem = { label: separator[1].trim(), items: [], collapsed: true };
      root.push(group);
      current = group.items;
      continue;
    }

    const link = LINK.exec(name);
    if (link?.[1] && link[2]) {
      current.push({ label: link[1], link: link[2] });
      continue;
    }

    const sub = `${dir}/${name}`;
    const subMeta = readMeta(sub);
    if (subMeta) {
      current.push({
        label: subMeta.title ?? name,
        collapsed: true,
        items: buildDir(sub),
      });
    } else if (name === "index") {
      current.push({ slug: dir });
    } else if (existsSync(join(DOCS_ROOT, `${sub}.mdx`))) {
      current.push({ slug: sub });
    } else {
      console.warn(`[sidebar] meta.json in ${dir} lists "${name}" but no such page exists`);
    }
  }
  return root;
}

export function buildSidebar() {
  return buildDir(`docs/${LATEST_SLUG}`);
}
