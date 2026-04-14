import type { CommandMap, EventMap } from "../src/types";
import type { BunviewClient, ClientOptions, RpcProxy } from "./types";
import { serialize, deserialize } from "../src/binary";

export type { BunviewClient, ClientOptions, RpcProxy };
export type { CommandMap, EventMap, ClientCommands } from "../src/types";
export { Channel } from "./channel";

const DEFAULT_TIMEOUT = 30_000;

/** Create a typed RPC client. Use `client.rpc.cmd(arg)` to call backend commands. */
export function createClient<
  TCommands extends CommandMap = CommandMap,
  TEvents extends EventMap = EventMap,
>(options?: ClientOptions): BunviewClient<TCommands, TEvents> {
  if (typeof window === "undefined") {
    throw new Error("[bunview] createClient() must be called in a browser/webview context.");
  }

  const api = window.__bunview_api__;
  if (!api) {
    throw new Error(
      "[bunview] window.__bunview_api__ not found.\n" +
      "         Make sure your HTML is served through the Bunview asset server.",
    );
  }

  const timeout = options?.timeout ?? DEFAULT_TIMEOUT;

  const invoke = (command: string, payload?: unknown): Promise<unknown> => {
    // Encode Uint8Array / ArrayBuffer in payload; decode typed arrays in response.
    const rpc = api.invoke(command, serialize(payload)).then((result) => deserialize(result));
    if (timeout <= 0) return rpc;
    return Promise.race([
      rpc,
      new Promise<never>((_, reject) =>
        setTimeout(
          () => reject(new Error(`[bunview] invoke("${command}") timed out after ${timeout}ms`)),
          timeout,
        ),
      ),
    ]);
  };

  const rpc = new Proxy({} as RpcProxy<TCommands>, {
    get(_t, prop: string | symbol) {
      if (typeof prop === "symbol") return undefined;
      return (payload?: unknown) => invoke(prop, payload);
    },
  });

  return {
    rpc,
    invoke: invoke as BunviewClient<TCommands, TEvents>["invoke"],
    listen: (event, handler) => api.listen(event, handler as (p: unknown) => void),
    once:   (event, handler) => api.once(event, handler as (p: unknown) => void),
    emit:   (event, payload) => api.emit(event, payload),
    onConnectionChange: (handler) => api.onConnectionChange(handler),
    get isConnected() { return api.isConnected(); },
  };
}
