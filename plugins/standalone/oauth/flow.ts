import type { OAuthStartResult } from "./types";

/** Active OAuth listener handle returned by `oauth.start`. */
export interface OAuthFlow {
  id: number;
  port: number;
  redirectUri: string;
  /** Resolves with the parsed query params from the redirect. Rejects on
   *  `cancel()` or after `timeoutMs` (default 5 min). */
  wait(opts?: { timeoutMs?: number }): Promise<{ params: Record<string, string> }>;
  cancel(): Promise<void>;
}

interface CallbackPayload {
  id?: number;
  params?: Record<string, string>;
}

interface ErrorPayload {
  id?: number;
  error?: string;
}

/** Runtime hooks kept explicit so flow lifecycle can be tested without global module mocks. */
export interface FlowDependencies {
  cancel(id: number): Promise<void>;
  onCallback(handler: (payload: CallbackPayload) => void): () => void;
  onError(handler: (payload: ErrorPayload) => void): () => void;
}

type FlowResult = { params: Record<string, string> };
type FlowOutcome =
  | { state: "pending" }
  | { state: "resolved"; value: FlowResult }
  | { state: "rejected" | "cancelled"; error: Error };

interface FlowWaiter {
  resolve(value: FlowResult): void;
  reject(error: Error): void;
  timer?: ReturnType<typeof setTimeout>;
}

class OAuthFlowController {
  private readonly waiters = new Set<FlowWaiter>();
  private outcome: FlowOutcome = { state: "pending" };
  private cancelPromise: Promise<void> | null = null;
  private offCallback: () => void = () => undefined;
  private offError: () => void = () => undefined;

  private readonly handle: OAuthStartResult;
  private readonly dependencies: FlowDependencies;

  constructor(handle: OAuthStartResult, dependencies: FlowDependencies) {
    this.handle = handle;
    this.dependencies = dependencies;
    this.offCallback = dependencies.onCallback((payload) => this.onCallback(payload));
    this.offError = dependencies.onError((payload) => this.onError(payload));
    if (this.outcome.state !== "pending") this.cleanupListeners();
  }

  toFlow(): OAuthFlow {
    return {
      id: this.handle.id,
      port: this.handle.port,
      redirectUri: this.handle.redirectUri,
      wait: (opts) => this.wait(opts?.timeoutMs),
      cancel: () => this.cancel(),
    };
  }

  wait(timeoutMs = 5 * 60 * 1000): Promise<FlowResult> {
    if (this.outcome.state === "resolved") return Promise.resolve(this.outcome.value);
    if (this.outcome.state !== "pending") return Promise.reject(this.outcome.error);

    return new Promise((resolve, reject) => {
      const waiter: FlowWaiter = { resolve, reject };
      this.waiters.add(waiter);
      waiter.timer = setTimeout(() => this.timeout(waiter, timeoutMs), timeoutMs);
    });
  }

  cancel(): Promise<void> {
    if (this.outcome.state !== "pending") return Promise.resolve();
    if (this.cancelPromise) return this.cancelPromise;
    this.cancelPromise = this.dependencies.cancel(this.handle.id).then(
      () => this.settle({ state: "cancelled", error: new Error("oauth: flow cancelled") }),
      (error: unknown) => {
        this.cancelPromise = null;
        throw error;
      },
    );
    return this.cancelPromise;
  }

  private onCallback(payload: CallbackPayload): void {
    if (payload.id !== this.handle.id) return;
    this.settle({ state: "resolved", value: { params: payload.params ?? {} } });
  }

  private onError(payload: ErrorPayload): void {
    if (payload.id !== this.handle.id) return;
    this.settle({
      state: "rejected",
      error: new Error(`oauth: ${payload.error ?? "listener error"}`),
    });
  }

  private timeout(waiter: FlowWaiter, timeoutMs: number): void {
    if (!this.waiters.delete(waiter)) return;
    this.cancel().catch(() => undefined);
    waiter.reject(new Error(`oauth: timed out after ${timeoutMs}ms`));
  }

  private settle(next: Exclude<FlowOutcome, { state: "pending" }>): void {
    if (this.outcome.state !== "pending") return;
    this.outcome = next;
    this.cleanupListeners();
    for (const waiter of this.waiters) {
      if (waiter.timer !== undefined) clearTimeout(waiter.timer);
      if (next.state === "resolved") waiter.resolve(next.value);
      else waiter.reject(next.error);
    }
    this.waiters.clear();
  }

  private cleanupListeners(): void {
    this.offCallback();
    this.offError();
  }
}

/** Build one OAuth flow and keep every waiter tied to its cancellation lifecycle. */
export function createOAuthFlow(
  handle: OAuthStartResult,
  dependencies: FlowDependencies,
): OAuthFlow {
  return new OAuthFlowController(handle, dependencies).toFlow();
}
