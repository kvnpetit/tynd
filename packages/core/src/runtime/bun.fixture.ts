/** Backend entry module used by bun.test.ts (`TYND_ENTRY`). */
import { getCallContext } from "./context";

export async function whoCalled(): Promise<unknown> {
  await Promise.resolve();
  await Bun.sleep(1);
  return getCallContext() ?? null;
}

/** Must never be reachable over IPC: `__tynd_*__` names are framework-reserved. */
export function __tynd_admin__(): string {
  return "pwned";
}

export function add(a: number, b: number): number {
  return a + b;
}

export async function* countdown(): AsyncGenerator<unknown, string> {
  for (let i = 0; i < 2; i++) {
    await Bun.sleep(1);
    yield getCallContext()?.window ?? null;
  }
  return "done";
}
