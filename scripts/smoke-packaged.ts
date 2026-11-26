import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const root = realpathSync(resolve(import.meta.dir, ".."));
const target = resolve(root, Bun.env.CARGO_TARGET_DIR ?? "target", "debug");
const extension = process.platform === "win32" ? ".exe" : "";
const testNativeMenus = process.platform === "darwin" || process.platform === "win32";
const nativeMenuImports = `import { menu } from "../../plugins/host/menu/client";
import { tray } from "../../plugins/host/tray/client";`;
const nativeMenuChecks = `
  await menu.setMenu([{
    type: "submenu", label: "Smoke", items: [{ id: "action", label: "Action" }],
  }]);
  await menu.setItemEnabled("action", false);
  const status = await tray.create("smoke", {
    icon: { rgba: new Uint8Array([0, 0, 0, 255]), width: 1, height: 1 },
    menu: [{ type: "checkbox", id: "check", label: "Check" }],
  });
  await status.setItemChecked("check", true);
  await status.remove();`;
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
    JSON.stringify({
      backend: "backend.ts",
      frontendDir: "frontend",
      bundle: { identifier: "com.tynd.smoke" },
      ...(testNativeMenus ? { apis: ["menu", "tray"] } : {}),
    }),
  );
  writeFileSync(
    join(project, "backend.ts"),
    `import { app } from "@tynd/core";

export function ping() { return "tynd-ready"; }
export async function confirm(reply: string) {
  if (reply !== "tynd-ready") throw new Error("Unexpected ping result: " + reply);
  await Bun.write(process.env.TYND_SMOKE_RESULT!, reply);
}
export async function reportFailure(message: string) {
  await Bun.write(process.env.TYND_SMOKE_RESULT!, message);
}
app.start({ window: { title: "Tynd packaged smoke" } });
`,
  );
  const client = join(project, "client.ts");
  writeFileSync(
    client,
    `import { createBackend } from "@tynd/core/client";
${testNativeMenus ? nativeMenuImports : ""}

const backend = createBackend<typeof import("./backend")>();
try {${testNativeMenus ? nativeMenuChecks : ""}
  await backend.confirm(await backend.ping());
} catch (error) {
  await backend.reportFailure(String(error));
}
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
  assert.ok(await Bun.file(result).exists(), "WebView did not complete the smoke checks");
  assert.equal(await Bun.file(result).text(), "tynd-ready");
  process.stdout.write("Packaged app loaded its frontend and completed a backend RPC.\n");
} finally {
  rmSync(project, { recursive: true, force: true });
}
