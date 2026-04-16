import path from "node:path"
import { defineConfig } from "vite"

const CORE = path.resolve(import.meta.dir, "../core")

export default defineConfig({
  root: CORE,
  server: { port: 5174 },
  define: {
    "globalThis.__VORN_RUNTIME__": JSON.stringify("lite"),
  },
  build: {
    outDir: path.resolve(import.meta.dir, "dist"),
    emptyOutDir: true,
  },
})
