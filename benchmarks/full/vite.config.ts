import { defineConfig } from "vite"
import path from "path"

const CORE = path.resolve(import.meta.dir, "../core")

export default defineConfig({
  root: CORE,
  server: { port: 5174 },
  define: {
    "globalThis.__VORN_RUNTIME__": JSON.stringify("full"),
  },
  build: {
    outDir:      path.resolve(import.meta.dir, "dist"),
    emptyOutDir: true,
  },
})
