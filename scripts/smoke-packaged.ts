import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const root = realpathSync(resolve(import.meta.dir, ".."));
const target = resolve(root, Bun.env.CARGO_TARGET_DIR ?? "target", "debug");
const extension = process.platform === "win32" ? ".exe" : "";
const parent = join(root, ".tynd");
mkdirSync(parent, { recursive: true });
assert.equal(realpathSync(parent), parent, "smoke directory must be inside the repository");
const project = realpathSync(mkdtempSync(join(parent, "smoke-packaged-")));
assert.equal(dirname(project), parent, "unexpected smoke project path");

try {
  const frontend = join(project, "frontend");
  mkdirSync(frontend);
  writeFileSync(join(project, "package.json"), '{"name":"tynd-smoke","private":true}');
  writeFileSync(
    join(project, "tynd.config.json"),
    '{"backend":"backend.ts","frontendDir":"frontend","bundle":{"identifier":"com.tynd.smoke"}}',
  );
  writeFileSync(
    join(project, "backend.ts"),
    `import { app } from "@tynd/core";

export function ping() { return "tynd-ready"; }
export async function confirm(reply: string) {
  if (reply !== "tynd-ready") throw new Error("Unexpected ping result: " + reply);
  await Bun.write(process.env.TYND_SMOKE_RESULT!, reply);
}
app.start({ window: { title: "Tynd packaged smoke" } });
`,
  );
  const client = join(project, "client.ts");
  writeFileSync(
    client,
    `import { createBackend } from "@tynd/core/client";

const backend = createBackend<typeof import("./backend")>();
await backend.confirm(await backend.ping());
`,
  );
  await Bun.build({ entrypoints: [client], outdir: frontend, target: "browser", throw: true });
  writeFileSync(
    join(frontend, "index.html"),
    '<!doctype html><html lang="en"><title>Tynd smoke</title><body><script type="module" src="./client.js"></script></body></html>',
  );

  const executable = join(project, `tynd-smoke${extension}`);
  const build = Bun.spawn([join(target, `tynd${extension}`), "build", "--out", executable], {
    cwd: project,
    env: { ...process.env, TYND_BUNDLER_BIN: join(target, `tynd-bundler${extension}`) },
    stdout: "inherit",
    stderr: "inherit",
  });
  assert.equal(await build.exited, 0, "tynd build failed");

  const result = join(project, "result.txt");
  const app = Bun.spawn([executable], {
    cwd: project,
    env: {
      ...process.env,
      TYND_PACKAGED_SMOKE: "1",
      TYND_SMOKE_DURATION_MS: "10000",
      TYND_SMOKE_RESULT: result,
    },
    stdout: "inherit",
    stderr: "inherit",
    signal: AbortSignal.timeout(60_000),
  });
  assert.equal(await app.exited, 0, "packaged app failed or timed out");
  assert.ok(await Bun.file(result).exists(), "WebView did not confirm the backend RPC response");
  assert.equal(await Bun.file(result).text(), "tynd-ready");
  process.stdout.write("Packaged app loaded its frontend and completed a backend RPC.\n");
} finally {
  rmSync(project, { recursive: true, force: true });
}
