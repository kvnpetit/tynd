/** Async function exposed to the frontend as `client.rpc.<name>`. */
export type RPCHandler<TInput = any, TOutput = any> = (
  input: TInput,
) => TOutput | Promise<TOutput>;

/** Map of named RPC handlers — typically `typeof import("./commands")`. */
export type CommandMap = Record<string, RPCHandler<any, any>>;

/** Map of named events flowing on the `emit/listen` channel. */
export type EventMap = Record<string, any>;

export type InferInput<T extends RPCHandler>  = T extends RPCHandler<infer I, any> ? I : never;
export type InferOutput<T extends RPCHandler> = Awaited<ReturnType<T>>;

/** Client-side type of a command module — Promise-wrapped returns. */
export type ClientCommands<T extends CommandMap> = {
  [K in keyof T]: (input: InferInput<T[K]>) => Promise<InferOutput<T[K]>>;
};
