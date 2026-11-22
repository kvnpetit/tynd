import { afterAll, describe, expect, test } from "bun:test";
import { cpSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { mdxToMarkdown, syncCatalog, targets } from "./gen-mcp-catalog";

const ROOT = path.resolve(import.meta.dir, "..");

describe("mdxToMarkdown", () => {
  test("drops component imports and collapses the gap after the frontmatter", () => {
    const source = [
      "---",
      "title: Page",
      "---",
      "",
      "import { Aside } from '@astrojs/starlight/components';",
      "",
      "# Page",
      "",
    ].join("\n");
    expect(mdxToMarkdown(source)).toBe("---\ntitle: Page\n---\n\n# Page\n");
  });

  test("maps asides to callouts and makes docs links absolute", () => {
    const source = [
      '<Aside type="tip">',
      "See [IPC](/docs/v1.0/concepts/ipc).",
      "</Aside>",
      '<Aside type="caution">careful</Aside>',
    ].join("\n");
    expect(mdxToMarkdown(source)).toBe(
      [
        '<Callout type="info">',
        "See [IPC](https://tynd-apps.github.io/tynd/docs/v1.0/concepts/ipc).",
        "</Callout>",
        '<Callout type="warning">careful</Callout>',
      ].join("\n"),
    );
  });

  test("renames fence titles and leaves fenced code alone", () => {
    const source = [
      '```ts title="backend/main.ts"',
      "import { app } from '@tynd/core';",
      '<Aside type="note">',
      "---",
      "```",
    ].join("\n");
    expect(mdxToMarkdown(source)).toBe(
      [
        '```ts filename="backend/main.ts"',
        "import { app } from '@tynd/core';",
        '<Aside type="note">',
        "---",
        "```",
      ].join("\n"),
    );
  });
});

describe("mdxToMarkdown line endings", () => {
  test("normalizes CRLF sources", () => {
    expect(mdxToMarkdown("---\r\ntitle: Page\r\n---\r\n\r\n# Page\r\n")).toBe(
      "---\ntitle: Page\n---\n\n# Page\n",
    );
  });
});

describe("syncCatalog", () => {
  const sandbox = mkdtempSync(path.join(tmpdir(), "tynd-mcp-catalog-"));
  afterAll(() => rmSync(sandbox, { recursive: true, force: true }));

  for (const target of targets()) {
    for (const file of [target.source, target.file]) {
      mkdirSync(path.dirname(path.join(sandbox, file)), { recursive: true });
      cpSync(path.join(ROOT, file), path.join(sandbox, file));
    }
  }

  test("every source exists and the committed catalog is up to date", () => {
    for (const target of targets()) expect(existsSync(path.join(ROOT, target.source))).toBe(true);
    expect(syncCatalog(ROOT, true)).toEqual([]);
  });

  test("--check reports drifted files without writing, then a run repairs them", () => {
    const doc = "packages/mcp/catalog/framework-docs/ipc.md";
    const schema = "packages/mcp/catalog/plugin-schemas/audio.gen.ts";
    writeFileSync(path.join(sandbox, doc), "stale\n");
    writeFileSync(path.join(sandbox, schema), "stale\n");

    expect(syncCatalog(sandbox, true)).toEqual([doc, schema]);
    expect(syncCatalog(sandbox, true)).toEqual([doc, schema]);
    expect(syncCatalog(sandbox, false)).toEqual([doc, schema]);
    expect(syncCatalog(sandbox, true)).toEqual([]);
  });

  test("a missing source is an error", () => {
    rmSync(path.join(sandbox, "plugins/standalone/tts/tts.gen.ts"));
    expect(() => syncCatalog(sandbox, true)).toThrow("missing source");
  });
});
