// Markdown links/images are written as root-relative paths (`/docs/v1.0/...`).
// GitHub Pages serves the site under `/<repo>`, so prefix them with the base and
// give page links a trailing slash (directory output) to avoid a 301 per click.
type HastNode = {
  type: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
};

const ATTR_BY_TAG: Record<string, string> = { a: "href", img: "src", source: "src" };

/** Root-relative and not already under the base (protocol-relative `//` URLs are external). */
function needsBase(url: string, base: string): boolean {
  if (!url.startsWith("/") || url.startsWith("//")) return false;
  return url !== base && !url.startsWith(`${base}/`);
}

function withTrailingSlash(url: string): string {
  const match = url.match(/^([^?#]*)(.*)$/);
  let path = match?.[1] ?? url;
  const rest = match?.[2] ?? "";
  const last = path.split("/").pop() ?? "";
  if (!path.endsWith("/") && !last.includes(".")) path += "/";
  return `${path}${rest}`;
}

function rewriteNode(node: HastNode, base: string) {
  const attr = node.tagName ? ATTR_BY_TAG[node.tagName] : undefined;
  const value = attr ? node.properties?.[attr] : undefined;
  if (!attr || !node.properties || typeof value !== "string") return;
  if (!needsBase(value, base)) return;
  // Only page links get a trailing slash; images/assets keep their path.
  node.properties[attr] = `${base}${node.tagName === "a" ? withTrailingSlash(value) : value}`;
}

export default function rehypeBaseLinks({ base }: { base: string }) {
  const prefix = base.replace(/\/$/, "");
  const visit = (node: HastNode) => {
    if (node.type === "element") rewriteNode(node, prefix);
    for (const child of node.children ?? []) visit(child);
  };
  return (tree: HastNode) => visit(tree);
}
