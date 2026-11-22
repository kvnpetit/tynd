/** Subprocess entry for the boot-order test in bun.test.ts: boots the real `startBun`. */
import { startBun } from "./bun";

(globalThis as Record<string, unknown>).__TYND_APP_INFO__ = { version: "9.9.9" };
(globalThis as Record<string, unknown>).__TYND_UPDATER_CONFIG__ = {
  endpoints: ["https://e.example/m.json"],
  publicKeys: ["PUB"],
  channel: "stable",
};
startBun({});
