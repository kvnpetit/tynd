# @tynd/test

End-to-end test harness for Tynd apps. It launches your app (`tynd dev`, debug socket on), drives
the webview over the Tynd debug protocol, and works with `bun test`. No runtime dependencies.

```ts
import { afterAll, beforeAll, expect, test } from "bun:test";
import { launchApp, type TyndApp } from "@tynd/test";

let app: TyndApp;
beforeAll(async () => {
  app = await launchApp(); // cwd defaults to TYND_TEST_PROJECT, then process.cwd()
});
afterAll(() => app.close());

test("greets the user", async () => {
  await app.fill([{ selector: "#name", value: "Ada" }]);
  await app.click("button[type=submit]");
  await app.waitFor(async () => (await app.eval<string>("document.title")) === "Hello Ada");
  await app.expectScreenshot("e2e/__screenshots__/greeting.png", { maxDiffRatio: 0.001 });
});
```

Run with `tynd test` (sets `TYND_TEST_CLI`, `TYND_TEST_PROJECT` and a 180 s per-test timeout) or plain
`bun test`. Screenshot baselines are created when missing; `tynd test --update-snapshots` (or
`TYND_UPDATE_SNAPSHOTS=1`) rewrites them. On a mismatch `<name>.actual.png` and `<name>.diff.png` are
written next to the baseline.

API: `launchApp`, `TyndApp` (typed `windowList`, `eval`, `click`, `type`, `fill`, `pressKey`,
`snapshot`, `screenshot`, `findElement`, `consoleLogs`, `appInfo`, `ipcRecent`, `waitFor`,
`waitForSelector`, `expectScreenshot`, `close`), `waitFor` (throws `WaitForTimeoutError`), `diffPng`,
`expectScreenshot`, and the lower level `DebugClient`/`connectTransport`. The `@tynd/test/png` and
`@tynd/test/protocol` subpaths expose the PNG helpers and the debug-protocol types.

By default `launchApp` runs `tynd dev`. To drive a packaged binary with a custom `command`, build it
with `tynd build --debug-protocol`; packaged apps refuse to start the debug server otherwise.

Screenshots (`screenshot.window`) need a host built with the `debug-screenshot` feature.

Guides: [Testing](https://tynd-apps.github.io/tynd/docs/v1.0/guides/testing) and [`tynd test`](https://tynd-apps.github.io/tynd/docs/v1.0/cli/test).
