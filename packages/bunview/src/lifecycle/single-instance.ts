import path from "path";
import fs from "fs";
import os from "os";
import type { SecondInstancePayload } from "../types";

export interface LockResult {
  acquired: boolean;
  /** Primary watches this path; 2nd instances write their argv payload here. */
  triggerPath: string;
}

/** Reclaims stale locks (dead PID). On conflict, forwards argv via trigger file. */
export async function acquireLock(appName: string, argv: string[]): Promise<LockResult> {
  const safe        = appName.replace(/[^a-zA-Z0-9]/g, "-");
  const lockPath    = path.join(os.tmpdir(), `bunview-${safe}.lock`);
  const triggerPath = path.join(os.tmpdir(), `bunview-${safe}.trigger`);

  try {
    const content = fs.readFileSync(lockPath, "utf-8").trim();
    const pid = parseInt(content, 10);
    if (pid && pid !== process.pid) {
      try {
        process.kill(pid, 0);
        const payload: SecondInstancePayload = { argv, cwd: process.cwd(), timestamp: Date.now() };
        fs.writeFileSync(triggerPath, JSON.stringify(payload));
        return { acquired: false, triggerPath };
      } catch { /* dead pid — reclaim */ }
    }
  } catch { /* first instance */ }

  fs.writeFileSync(lockPath, String(process.pid));
  try { fs.writeFileSync(triggerPath, ""); } catch {}

  const cleanup = () => {
    try { fs.unlinkSync(lockPath); } catch {}
    try { fs.unlinkSync(triggerPath); } catch {}
  };
  process.on("exit", cleanup);
  process.on("SIGINT",  () => { cleanup(); process.exit(0); });
  process.on("SIGTERM", () => { cleanup(); process.exit(0); });

  return { acquired: true, triggerPath };
}

/** Poll the trigger file; truncate on receive, fire `onPayload` with parsed content. */
export function watchTrigger(
  triggerPath: string,
  onPayload: (payload: SecondInstancePayload) => void | Promise<void>,
): void {
  fs.watchFile(triggerPath, { interval: 250, persistent: false }, async () => {
    let content: string;
    try { content = fs.readFileSync(triggerPath, "utf-8"); } catch { return; }
    if (!content.trim()) return;

    let payload: SecondInstancePayload;
    try { payload = JSON.parse(content); } catch { return; }

    await onPayload(payload);
    try { fs.writeFileSync(triggerPath, ""); } catch {}
  });
}
