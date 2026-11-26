import { connect, type Socket } from "node:net";
import type { Transport } from "./client.ts";

/** Where a debug server listens: a Unix socket path / Windows named pipe, or a loopback TCP port. */
export type Endpoint = { unix: string } | { tcp: number; host?: string };

/** Adapt a `node:net` socket to the client {@link Transport}. */
export function socketTransport(socket: Socket): Transport {
  return {
    write: (data) => {
      socket.write(data);
    },
    onData: (listener) => {
      socket.on("data", (chunk: Buffer) => listener(chunk));
    },
    onClose: (listener) => {
      let reported = false;
      const report = (error?: Error) => {
        if (reported) return;
        reported = true;
        listener(error);
      };
      socket.on("error", (error) => report(error));
      socket.on("close", () => report());
    },
    close: () => {
      socket.destroy();
    },
  };
}

/** Open a connection to `endpoint`, retrying until `timeout` ms have passed. */
export async function connectTransport(endpoint: Endpoint, timeout = 5_000): Promise<Transport> {
  const deadline = Date.now() + timeout;
  let lastError: Error | undefined;
  do {
    try {
      const socket = await open(endpoint);
      return socketTransport(socket);
    } catch (error) {
      lastError = error instanceof Error ? error : new Error(String(error));
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  } while (Date.now() < deadline);
  throw new Error(`cannot connect to the debug endpoint: ${lastError?.message ?? "unknown error"}`);
}

function open(endpoint: Endpoint): Promise<Socket> {
  return new Promise((resolve, reject) => {
    const socket =
      "unix" in endpoint
        ? connect({ path: endpoint.unix })
        : connect({ host: endpoint.host ?? "127.0.0.1", port: endpoint.tcp });
    const onError = (error: Error) => {
      socket.destroy();
      reject(error);
    };
    socket.once("error", onError);
    socket.once("connect", () => {
      socket.off("error", onError);
      socket.setNoDelay?.(true);
      resolve(socket);
    });
  });
}
