// Starlight renders the frontmatter `title` as the page <h1>. The migrated pages also start
// with a markdown `# Title`, which produced a duplicate heading; drop that leading h1.
type HastNode = { type: string; tagName?: string; children?: HastNode[] };

export default function rehypeDropH1() {
  return (tree: HastNode) => {
    const children = tree.children;
    if (!children) return;
    const index = children.findIndex((n) => n.type === "element" && n.tagName === "h1");
    if (index !== -1) children.splice(index, 1);
  };
}
