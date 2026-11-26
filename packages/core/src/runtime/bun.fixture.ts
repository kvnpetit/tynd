/** Backend entry module used by bun.test.ts (`TYND_ENTRY`). */
import { getCallContext } from "./context";

/** The caller identity, without the (unencodable) abort signal. */
export function identity(): unknown {
  const ctx = getCallContext();
  if (!ctx) return null;
  const { signal, ...rest } = ctx;
  return { ...rest, aborted: signal.aborted };
}

export async function whoCalled(): Promise<unknown> {
  await Promise.resolve();
  await Bun.sleep(1);
  return identity();
}

/** Resolves when its call is cancelled, recording what the handler saw. */
export const aborts: { reason: unknown }[] = [];
export function waitForAbort(): Promise<string> {
  const signal = getCallContext()?.signal;
  return new Promise((resolve) => {
    signal?.addEventListener("abort", () => {
      aborts.push({ reason: signal.reason });
      resolve("too late");
    });
  });
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

/** Pushes two messages to a frontend channel, then closes it. */
export function pushTwice(label: string, channel: import("./channel").Channel<string>): string {
  channel.send(`${label}-1`);
  channel.send(`${label}-2`);
  channel.close();
  return channel.closed ? "closed" : "open";
}
