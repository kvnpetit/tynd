export function templateConfig(outDir: string): string {
  return `import { defineConfig } from "bunview";

export default defineConfig({
  entry: "backend/main.ts",
  frontend: "${outDir}",
});
`;
}

export function templateCommands(): string {
  return `// Every function exported from this module is callable from the frontend
// via \`client.rpc.<name>(arg)\`. Keep backend-only helpers in \`./internal.ts\` —
// they are invisible to the webview by construction.

export async function greet(name: string) {
  return \`Hello, \${name}!\`;
}
`;
}

export function templateInternal(): string {
  return `// Backend-only helpers. NOT exposed to the frontend — this file is never
// passed to \`createApp({ commands })\`, so the IPC bridge cannot reach it.

export async function cleanup() {
  // e.g. flush caches, close DB connections, …
}
`;
}

export function templateBackendTsconfig(): string {
  return `{
  "compilerOptions": {
    "target": "es2023",
    "lib": ["ES2023"],
    "module": "esnext",
    "moduleResolution": "bundler",
    "types": ["bun-types"],
    "skipLibCheck": true,
    "noEmit": true,
    "allowImportingTsExtensions": true,
    "isolatedModules": true,
    "esModuleInterop": true,
    "strict": true
  },
  "include": ["./**/*.ts"]
}
`;
}

export function templateBackend(name: string, outDir: string): string {
  return `import { createApp } from "bunview";
import * as commands from "./commands";
import { cleanup } from "./internal";

export type AppCommands = typeof commands;

const app = createApp({
  entry: "${outDir}",
  commands,
  window: { title: "${name}", width: 1024, height: 768 },
});

app.onReady(() => console.log("[${name}] window ready"));
app.onClose(() => { cleanup(); console.log("[${name}] window closed"); });

await app.run();
`;
}

export function templateBackendVanilla(name: string): string {
  return `import { createApp } from "bunview";
import * as commands from "./commands";
import { cleanup } from "./internal";

export type AppCommands = typeof commands;

const app = createApp({
  entry: "./frontend",
  commands,
  window: { title: "${name}", width: 900, height: 600 },
});

app.onReady(() => console.log("[${name}] window ready"));
app.onClose(() => { cleanup(); console.log("[${name}] window closed"); });

await app.run();
`;
}

export function templateHtml(name: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>${name}</title>
  <style>
    body { font-family: system-ui, sans-serif; background: #0f0f13; color: #e8e8f0; display: grid; place-items: center; min-height: 100vh; margin: 0; }
    main { text-align: center; }
    h1 { font-size: 2rem; background: linear-gradient(135deg, #a89af9, #7c6af7); -webkit-background-clip: text; -webkit-text-fill-color: transparent; }
    #result { margin-top: 1rem; color: #888; }
    button { background: #7c6af7; color: #fff; border: none; border-radius: 6px; padding: 8px 20px; cursor: pointer; font-size: 14px; }
    button:hover { opacity: .85; }
  </style>
</head>
<body>
  <main>
    <h1>${name}</h1>
    <p>Powered by Bunview</p>
    <button id="btn">Say Hello</button>
    <p id="result"></p>
  </main>
  <script type="module" src="app.js"></script>
</body>
</html>
`;
}

export function templateAppJs(): string {
  return `// bunview client is injected automatically via /__bunview__/client.js
// Access it via window.__bunview_api__

document.getElementById("btn").addEventListener("click", async () => {
  const api = window.__bunview_api__;
  const msg = await api.invoke("greet", "World");
  document.getElementById("result").textContent = msg;
});
`;
}

export function templatePackageJson(name: string): string {
  return JSON.stringify({
    name,
    version: "0.1.0",
    type: "module",
    scripts: {
      dev:   "bunview dev",
      start: "bunview start",
      build: "bunview build",
    },
    dependencies: { bunview: "latest" },
    devDependencies: { "bun-types": "latest" },
  }, null, 2);
}
