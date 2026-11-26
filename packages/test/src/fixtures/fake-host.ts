/**
 * Stand-in for a Tynd host used by the launch tests: it opens a TCP debug server, publishes a
 * discovery file the way crates/host/src/debug.rs does, and answers a few debug methods.
 *
 * Environment: TYND_TEST_DISCOVERY_DIR (where to publish). Behavior flags via argv:
 *   --no-window   window.list stays empty (readiness never completes)
 *   --exit-early  exit with code 3 before publishing anything
 *   --ignore-socket  listen on TCP even when TYND_DEBUG_SOCKET is set (a host that ignores the request)
 *
 * Like the real host, it listens on TYND_DEBUG_SOCKET (Unix socket / named pipe) when that is set.
 */
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { join } from "node:path";
import { encodeFrame, FrameDecoder } from "../protocol.ts";

if (process.argv.includes("--exit-early")) {
  console.error("fake host: boom");
  process.exit(3);
}

const dir = process.env.TYND_TEST_DISCOVERY_DIR;
if (!dir) throw new Error("TYND_TEST_DISCOVERY_DIR is required");
const token = "fake-token-1234567890";
const noWindow = process.argv.includes("--no-window");
const requestedSocket = process.argv.includes("--ignore-socket")
  ? undefined
  : process.env.TYND_DEBUG_SOCKET;

const server = createServer((socket) => {
  const decoder = new FrameDecoder();
  let authed = false;
  const send = (message: unknown) =>
    socket.write(encodeFrame(new TextEncoder().encode(JSON.stringify(message))));
  socket.on("data", (chunk: Buffer) => {
    for (const frame of decoder.push(chunk)) {
      const request = JSON.parse(new TextDecoder().decode(frame)) as {
        id: string;
        method: string;
        params: Record<string, unknown>;
      };
      const reply = (result: unknown) => send({ jsonrpc: "2.0", id: request.id, result });
      if (!authed) {
        if (request.method !== "bridge.hello" || request.params.cookie !== token) {
          send({ jsonrpc: "2.0", id: request.id, error: { code: -32602, message: "bad token" } });
          socket.end();
          return;
        }
        authed = true;
        reply({ cookie: token });
        continue;
      }
      switch (request.method) {
        case "window.list":
          reply(
            noWindow
              ? []
              : [
                  {
                    label: "main",
                    title: "Fake",
                    visible: true,
                    focused: true,
                    size: { width: 100, height: 80 },
                    position: { x: 0, y: 0 },
                  },
                ],
          );
          break;
        case "webview.eval":
          reply(request.params.script === "document.readyState" ? '"complete"' : "null");
          break;
        case "app.quit":
          reply({ ok: true });
          setTimeout(() => process.exit(0), 20);
          break;
        default:
          send({
            jsonrpc: "2.0",
            id: request.id,
            error: { code: -32601, message: `method not found: ${request.method}` },
          });
      }
    }
  });
});

const onListening = () => {
  let endpoint: { unix: string } | { tcp: number };
  if (requestedSocket) {
    endpoint = { unix: requestedSocket };
  } else {
    const address = server.address();
    if (!address || typeof address === "string") throw new Error("no address");
    endpoint = { tcp: address.port };
  }
  mkdirSync(dir, { recursive: true });
  const file = join(dir, `${process.pid}.json`);
  writeFileSync(
    file,
    JSON.stringify({
      pid: process.pid,
      appId: "dev.tynd.fake",
      appVersion: "0.0.0",
      endpoint,
      startedAt: new Date().toISOString(),
      cookie: token,
    }),
  );
  process.on("exit", () => rmSync(file, { force: true }));
  console.log("fake host ready");
};

if (requestedSocket) server.listen(requestedSocket, onListening);
else server.listen(0, "127.0.0.1", onListening);
