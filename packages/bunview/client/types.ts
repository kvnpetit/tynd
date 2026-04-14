import type { CommandMap, EventMap } from "../src/types";

export type { CommandMap, EventMap };

export interface ClientOptions {
  /** Timeout in milliseconds for invoke() calls. 0 = no timeout. Default: 30000 */
  timeout?: number;
}

/** Each backend command exposed as a typed direct method. */
export type RpcProxy<T extends CommandMap> = {
  [K in keyof T]: Parameters<T[K]> extends []
    ? () => Promise<Awaited<ReturnType<T[K]>>>
    : undefined extends Parameters<T[K]>[0]
      ? (payload?: Parameters<T[K]>[0]) => Promise<Awaited<ReturnType<T[K]>>>
      : (payload: Parameters<T[K]>[0]) => Promise<Awaited<ReturnType<T[K]>>>;
};

/** Backend RPC under `.rpc`, events/built-ins at the top level. */
export interface BunviewClient<
  TCommands extends CommandMap = CommandMap,
  TEvents extends EventMap = EventMap,
> {
  /** Proxy exposing every backend command as a typed method. */
  readonly rpc: RpcProxy<TCommands>;
  /** Subscribe to a typed backend event. Returns an unlisten function. */
  listen<K extends keyof TEvents & string>(
    event: K,
    handler: (payload: TEvents[K]) => void,
  ): () => void;
  /** Listen to a typed backend event once. Returns an unlisten function. */
  once<K extends keyof TEvents & string>(
    event: K,
    handler: (payload: TEvents[K]) => void,
  ): () => void;
  /** Emit a typed event to the backend. */
  emit<K extends keyof TEvents & string>(event: K, payload: TEvents[K]): void;
  /** Whether the event channel is currently connected. */
  readonly isConnected: boolean;
  /** Subscribe to connection state changes. Returns an unlisten function. */
  onConnectionChange(handler: (connected: boolean) => void): () => void;
  /** Dynamic dispatch: invoke a backend command by string name. */
  invoke<K extends keyof TCommands & string>(
    command: K,
    payload: Parameters<TCommands[K]>[0],
  ): Promise<Awaited<ReturnType<TCommands[K]>>>;
}

declare global {
  interface Window {
    /** Raw `webview_bind` function registered by the native host. */
    __bunview__: ((rpcJson: string) => Promise<unknown>) | undefined;
    __bunview_dispatch__: (name: string, payloadJson: string) => void;
    __bunview_api__: {
      invoke(command: string, payload: unknown): Promise<unknown>;
      listen(event: string, handler: (payload: unknown) => void): () => void;
      once(event: string, handler: (payload: unknown) => void): () => void;
      isConnected(): boolean;
      onConnectionChange(handler: (connected: boolean) => void): () => void;
      emit(event: string, payload: unknown): void;
    };
  }
}
