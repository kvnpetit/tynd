import { createEmitter, definePlugin, hostCall } from "@tynd/core";
import type { OAuthEventMap, OAuthIdArgs, OAuthStartArgs, OAuthStartResult } from "./types";

const MAX_FLOWS = 16;
const SUCCESS_HTML_MAX_BYTES = 4 * 1024;
const PATH_MAX_BYTES = 256;
const PARAM_VALUE_MAX_BYTES = 4 * 1024;
const STATE_MAX_BYTES = 4 * 1024;
const FLOW_TIMEOUT_MS = 10 * 60 * 1000;
const DEFAULT_HTML = `<!doctype html>
<html lang="en">
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width">
  <title>Authentication complete</title>
  <body>
    <h1>Authentication complete</h1>
    <p>You can close this tab and return to the app.</p>
  </body>
</html>`;

type Emit = <K extends keyof OAuthEventMap>(event: K, payload: OAuthEventMap[K]) => void;
type CheckBind = (port: number) => Promise<void>;

interface BackendDependencies {
  emit?: Emit;
  checkBind?: CheckBind;
}

interface FlowState {
  id: number;
  port: number;
  server: Bun.Server<undefined>;
  timer: ReturnType<typeof setTimeout>;
}

const events = createEmitter<OAuthEventMap>();

function defaultEmit<K extends keyof OAuthEventMap>(event: K, payload: OAuthEventMap[K]): void {
  events.emit(event, payload);
}

async function defaultCheckBind(port: number): Promise<void> {
  await hostCall("network-loopback", "bind", { port });
}

function byteLength(value: string): number {
  return Buffer.byteLength(value);
}

function validatePort(port: number): void {
  if (!Number.isInteger(port) || port < 0 || port > 65_535) {
    throw new TypeError("oauth.start: port must be an integer between 0 and 65535");
  }
}

export function validateOAuthPath(path: string): void {
  if (byteLength(path) > PATH_MAX_BYTES) {
    throw new TypeError(`oauth.start: path must be at most ${PATH_MAX_BYTES} bytes`);
  }
  if (!path.startsWith("/") || path.includes("//")) {
    throw new TypeError("oauth.start: path must start with one leading '/' and contain no '//'");
  }
  if (!/^[\x20-\x7e]+$/.test(path)) {
    throw new TypeError("oauth.start: path must be printable ASCII");
  }
  if (path.includes("?") || path.includes("#") || path.includes("\0")) {
    throw new TypeError("oauth.start: path must not contain '?', '#', or NUL");
  }
}

export function validateSuccessHtml(html: string): void {
  if (byteLength(html) > SUCCESS_HTML_MAX_BYTES) {
    throw new TypeError(`oauth.start: successHtml must be at most ${SUCCESS_HTML_MAX_BYTES} bytes`);
  }
  if ([...html].some((character) => character === "\0" || character.charCodeAt(0) > 0x7f)) {
    throw new TypeError("oauth.start: successHtml must be ASCII-only and contain no NUL");
  }
}

function validateExpectedState(state: string | null | undefined): void {
  if (state == null) return;
  if (
    byteLength(state) > STATE_MAX_BYTES ||
    state.includes("\0") ||
    [...state].some((character) => character.charCodeAt(0) < 0x20 || character.charCodeAt(0) > 0x7e)
  ) {
    throw new TypeError(
      `oauth.start: expectedState must be at most ${STATE_MAX_BYTES} bytes of printable ASCII`,
    );
  }
}

function sanitizeParamValue(value: string): string | null {
  if (value.includes("\0")) return null;
  let bytes = 0;
  let output = "";
  for (const character of value) {
    const size = byteLength(character);
    if (bytes + size > PARAM_VALUE_MAX_BYTES) break;
    if (!/^[\x20-\x7e]$/.test(character)) continue;
    output += character;
    bytes += size;
  }
  return output;
}

function safeParams(url: URL): Record<string, string> {
  const output: Record<string, string> = Object.create(null) as Record<string, string>;
  let count = 0;
  for (const [key, value] of url.searchParams) {
    if (count++ >= 128) break;
    const sanitizedKey = sanitizeParamValue(key);
    if (sanitizedKey === null) continue;
    const safeKey = sanitizedKey.slice(0, 256);
    if (!safeKey) continue;
    const safe = sanitizeParamValue(value);
    if (safe !== null) output[safeKey] = safe;
  }
  return output;
}

function response(body: string, status = 200, contentType = "text/plain; charset=utf-8"): Response {
  return new Response(body, {
    status,
    headers: {
      "content-type": contentType,
      "content-security-policy":
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'none'; frame-ancestors 'none'",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
    },
  });
}

function nextId(current: number, flows: Map<number, FlowState>): number {
  let candidate = current;
  for (let attempts = 0; attempts < Number.MAX_SAFE_INTEGER; attempts++) {
    candidate = candidate >= Number.MAX_SAFE_INTEGER ? 1 : candidate + 1;
    if (!flows.has(candidate)) return candidate;
  }
  throw new Error("oauth.start: no flow id available");
}

interface OAuthFetchContext {
  id: number;
  path: string;
  expectedState: string | null | undefined;
  successHtml: string;
  status: { ready: boolean; completed: boolean };
  emit: Emit;
  finish(id: number, force: boolean): Promise<void>;
}

function earlyOAuthResponse(context: OAuthFetchContext, request: Request): Response | null {
  if (!context.status.ready) return response("listener not ready", 503);
  if (context.status.completed) return response("callback already handled", 410);
  if (request.method !== "GET") return response("method not allowed", 405);
  return null;
}

function parseCallbackUrl(context: OAuthFetchContext, request: Request): URL | null {
  try {
    return new URL(request.url);
  } catch {
    context.emit("oauth:error", { id: context.id, error: "callback parse failed" });
    return null;
  }
}

function stateMismatch(context: OAuthFetchContext, url: URL): boolean {
  if (context.expectedState == null) return false;
  const states = url.searchParams.getAll("state");
  return states.length !== 1 || states[0] !== context.expectedState;
}

async function handleOAuthFetch(context: OAuthFetchContext, request: Request): Promise<Response> {
  const early = earlyOAuthResponse(context, request);
  if (early) return early;
  const url = parseCallbackUrl(context, request);
  if (!url) return response("bad request", 400);
  if (url.pathname !== context.path) return response("not found", 404);
  if (stateMismatch(context, url)) {
    context.status.completed = true;
    context.emit("oauth:error", { id: context.id, error: "state mismatch" });
    queueMicrotask(() => void context.finish(context.id, false));
    return response("state mismatch", 400);
  }
  context.status.completed = true;
  context.emit("oauth:callback", { id: context.id, params: safeParams(url) });
  queueMicrotask(() => void context.finish(context.id, false));
  return response(context.successHtml, 200, "text/html; charset=utf-8");
}

class OAuthBackendState {
  private readonly flows = new Map<number, FlowState>();
  private nextFlowId = 0;

  private readonly emit: Emit;
  private readonly checkBind: CheckBind;

  constructor(emit: Emit, checkBind: CheckBind) {
    this.emit = emit;
    this.checkBind = checkBind;
  }

  async start(args: OAuthStartArgs = {}): Promise<OAuthStartResult> {
    this.ensureCapacity();
    const requestedPort = args.port ?? 0;
    const path = args.path ?? "/";
    const successHtml = args.successHtml ?? DEFAULT_HTML;
    validatePort(requestedPort);
    validateOAuthPath(path);
    validateSuccessHtml(successHtml);
    validateExpectedState(args.expectedState);
    await this.checkBind(requestedPort);
    // The limit may have been reached by concurrent start() calls during the await.
    this.ensureCapacity();

    const id = this.allocateId();
    const status = { ready: false, completed: false };
    const server = Bun.serve({
      hostname: "127.0.0.1",
      port: requestedPort,
      fetch: (request) =>
        handleOAuthFetch(
          {
            id,
            path,
            expectedState: args.expectedState,
            successHtml,
            status,
            emit: this.emit,
            finish: (flowId, force) => this.finish(flowId, force),
          },
          request,
        ),
    });
    const port = server.port ?? requestedPort;
    const flow = { id, port, server, timer: this.scheduleTimeout(id) };
    this.flows.set(id, flow);
    await this.verifyBoundPort(flow, requestedPort);
    status.ready = true;
    return { id, port, redirectUri: `http://127.0.0.1:${port}${path}` };
  }

  async cancel({ id }: OAuthIdArgs): Promise<void> {
    await this.finish(id, true);
  }

  list(): number[] {
    return [...this.flows.keys()].sort((a, b) => a - b);
  }

  private ensureCapacity(): void {
    if (this.flows.size >= MAX_FLOWS) {
      throw new Error(`oauth.start: flow limit reached (max ${MAX_FLOWS})`);
    }
  }

  private allocateId(): number {
    this.nextFlowId = nextId(this.nextFlowId, this.flows);
    return this.nextFlowId;
  }

  private scheduleTimeout(id: number): ReturnType<typeof setTimeout> {
    return setTimeout(() => {
      this.emit("oauth:error", { id, error: "callback timed out" });
      this.finish(id, true).catch(() => undefined);
    }, FLOW_TIMEOUT_MS);
  }

  private async verifyBoundPort(flow: FlowState, requestedPort: number): Promise<void> {
    try {
      if (flow.port !== requestedPort) await this.checkBind(flow.port);
    } catch (error) {
      this.flows.delete(flow.id);
      clearTimeout(flow.timer);
      await flow.server.stop(true);
      throw error;
    }
  }

  private async finish(id: number, force: boolean): Promise<void> {
    const flow = this.flows.get(id);
    if (!flow) return;
    this.flows.delete(id);
    clearTimeout(flow.timer);
    await flow.server.stop(force);
  }
}

export function createOAuthMethods(dependencies: BackendDependencies = {}) {
  const state = new OAuthBackendState(
    dependencies.emit ?? defaultEmit,
    dependencies.checkBind ?? defaultCheckBind,
  );
  return {
    start: (args?: OAuthStartArgs) => state.start(args),
    cancel: (args: OAuthIdArgs) => state.cancel(args),
    list: () => state.list(),
  };
}

export const oauthBackend = definePlugin({
  name: "oauth",
  methods: createOAuthMethods(),
});

export default oauthBackend;
