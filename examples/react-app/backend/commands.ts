// Every function exported from this module is callable from the frontend via
// `client.rpc.<name>(…)`. Keep backend-only helpers in `./internal.ts` — they
// are invisible to the webview by construction (no config, no ACL to maintain).

import os from "os";

export async function greet(name: string) {
  return `Hello, ${name}! Running on ${os.platform()} ${os.arch()}.`;
}

export async function add(input: { a: number; b: number }) {
  return input.a + input.b;
}

export async function echoBytes(bytes: Uint8Array) {
  return bytes;
}

export async function getSystemInfo() {
  return {
    platform: os.platform(),
    arch: os.arch(),
    hostname: os.hostname(),
    cpus: os.cpus().length,
    totalMemoryGB: Math.round(os.totalmem() / 1024 ** 3 * 10) / 10,
    freeMemoryGB:  Math.round(os.freemem()  / 1024 ** 3 * 10) / 10,
    bunVersion: Bun.version,
  };
}

// Example: a file-association handler. To use, declare in bunview.config.ts:
//   fileAssociations: [{ ext: "md", name: "Markdown", handler: "openFile" }]
export async function openFile(filePath: string) {
  return Bun.file(filePath).text();
}
