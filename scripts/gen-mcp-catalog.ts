#!/usr/bin/env bun
/**
 * MCP catalog generator.
 *
 * `packages/mcp/catalog/` is a derived copy, never edited by hand:
 *   - framework-docs/<slug>.md      from apps/docs/src/content/docs/docs/v1.0/<page>.mdx
 *   - plugin-schemas/<name>.gen.ts  from plugins/standalone/<name>/<name>.gen.ts
 *
 * `crates/cli/build.rs` compiles every generated file into `tynd-mcp`, so `FRAMEWORK_DOCS` /
 * `PLUGIN_SCHEMAS` here are the only list of assets.
 *
 * Usage:
 *   bun scripts/gen-mcp-catalog.ts           rewrite the generated files
 *   bun scripts/gen-mcp-catalog.ts --check   exit 1 when any file is stale
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

export const DOCS_SOURCE_DIR = "apps/docs/src/content/docs/docs/v1.0";
export const DOCS_BASE_URL = "https://kvnpetit.github.io/tynd/docs/";
const CATALOG_DIR = "packages/mcp/catalog";

/** Catalog slug (stable MCP resource name) -> docs page, relative to `DOCS_SOURCE_DIR`. */
export const FRAMEWORK_DOCS: Record<string, string> = {
  architecture: "concepts/architecture",
  "auto-updates": "guides/auto-updates",
  "backend-rpc": "guides/backend-rpc",
  "build-pipeline": "concepts/build-pipeline",
  "code-signing": "guides/code-signing",
  debugging: "guides/debugging",
  "deep-linking": "guides/deep-linking",
  ipc: "concepts/ipc",
  "multi-window": "guides/multi-window",
  observability: "concepts/observability",
  runtimes: "concepts/runtimes",
  security: "concepts/security",
  troubleshooting: "troubleshooting",
  webview: "api/os/webview",
};

/** Official plugins whose generated schema ships in the catalog. */
export const PLUGIN_SCHEMAS = ["audio", "terminal"];

const IMPORT_LINE = /^import .* from '.*';?$/;
const ASIDE_TAGS: [string, string][] = [
  ['<Aside type="note">', '<Callout type="info">'],
  ['<Aside type="tip">', '<Callout type="info">'],
  ['<Aside type="caution">', '<Callout type="warning">'],
  ["</Aside>", "</Callout>"],
];

const normalizeEol = (text: string): string => text.replaceAll("\r\n", "\n");

const convertFenceOpener = (line: string): string =>
  line.replace(/\btitle="/, ' filename="').replaceAll("  filename=", " filename=");
const absoluteDocsLinks = (line: string): string =>
  line.replaceAll("](/docs/", `](${DOCS_BASE_URL}`);
const convertAsides = (line: string): string =>
  ASIDE_TAGS.reduce((text, [from, to]) => text.replaceAll(from, to), line);

/**
 * Convert a docs MDX page to the plain markdown the MCP server serves: drop component imports,
 * map `<Aside>` to `<Callout>`, rename code-fence `title=` to `filename=`, and make `/docs/`
 * links absolute.
 */
export function mdxToMarkdown(source: string): string {
  const out: string[] = [];
  let inFence = false;
  let frontmatterMarks = 0;
  for (const line of normalizeEol(source).split("\n")) {
    if (line.trim() === "---" && frontmatterMarks < 2 && !inFence) {
      frontmatterMarks += 1;
      out.push(line);
    } else if (line.trimStart().startsWith("```")) {
      out.push(inFence ? line : convertFenceOpener(line));
      inFence = !inFence;
    } else if (inFence) {
      out.push(absoluteDocsLinks(line));
    } else if (frontmatterMarks < 2 || !IMPORT_LINE.test(line)) {
      out.push(absoluteDocsLinks(convertAsides(line)));
    }
  }
  // Removing the imports leaves extra blank lines after the frontmatter.
  return out.join("\n").replace(/(---\n\n)\n+/, "$1");
}

export interface Target {
  /** Generated file, relative to the repo root. */
  file: string;
  /** Source file, relative to the repo root. */
  source: string;
  render: (source: string) => string;
}

export function targets(): Target[] {
  const docs = Object.entries(FRAMEWORK_DOCS).map(([slug, page]) => ({
    file: `${CATALOG_DIR}/framework-docs/${slug}.md`,
    source: `${DOCS_SOURCE_DIR}/${page}.mdx`,
    render: mdxToMarkdown,
  }));
  const schemas = PLUGIN_SCHEMAS.map((name) => ({
    file: `${CATALOG_DIR}/plugin-schemas/${name}.gen.ts`,
    source: `plugins/standalone/${name}/${name}.gen.ts`,
    render: normalizeEol,
  }));
  return [...docs, ...schemas];
}

/** Regenerate the catalog under `root`; returns the files that were (or, with `check`, are) stale. */
export function syncCatalog(root: string, check: boolean): string[] {
  const stale: string[] = [];
  for (const target of targets()) {
    const source = path.join(root, target.source);
    if (!existsSync(source)) throw new Error(`mcp catalog: missing source ${target.source}`);
    const file = path.join(root, target.file);
    const current = existsSync(file) ? readFileSync(file, "utf8") : "";
    const next = target.render(readFileSync(source, "utf8"));
    if (next === current) continue;
    stale.push(target.file);
    if (!check) writeFileSync(file, next);
  }
  return stale;
}

const out = (message: string) => process.stdout.write(`${message}\n`);
const fail = (message: string): never => {
  process.stderr.write(`${message}\n`);
  return process.exit(1);
};

function main(): void {
  const root = path.resolve(import.meta.dir, "..");
  const check = Bun.argv.includes("--check");
  let stale: string[] = [];
  try {
    stale = syncCatalog(root, check);
  } catch (error) {
    fail(error instanceof Error ? error.message : String(error));
  }
  if (check && stale.length > 0) {
    fail(
      `mcp catalog is stale; run \`bun run gen:mcp-catalog\`:\n${stale.map((file) => `  ${file}`).join("\n")}`,
    );
  }
  out(
    stale.length > 0
      ? `mcp catalog: updated ${stale.join(", ")}`
      : `mcp catalog is up to date (${Object.keys(FRAMEWORK_DOCS).length} docs + ${PLUGIN_SCHEMAS.length} schemas)`,
  );
}

if (import.meta.main) main();
