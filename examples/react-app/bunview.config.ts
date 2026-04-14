import { defineConfig } from "bunview";
import type * as commands from "./backend/commands";

// `<typeof commands>` unlocks compile-time checks: `handler` must be a command name,
// `allow`/`deny` autocomplete command names, and typos fail tsc.
export default defineConfig<typeof commands>({
  entry:    "backend/main.ts",
  frontend: "./dist",

  window: {
    title:  "Bunview React Example",
    width:  1000,
    height: 700,
  },

  windowState: true,

  urlScheme: {
    name: "bunview-example",
    // handler: "handleUrl",   // ← optional; typed against commands keys
  },

  fileAssociations: [
    { ext: "md", name: "Markdown File", handler: "openFile", mimeType: "text/markdown" },
    // { ext: "json", handler: "xxxxx" }  // ← tsc error: unknown command
  ],
});
