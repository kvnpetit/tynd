import { defineConfig } from "vite"

export default defineConfig({
  server: {
    port: 5174,
    host: "127.0.0.1",
    // Runtime is passed via URL param (?runtime=lite|full) at serve time.
    // __VORN_RUNTIME__ define is only meaningful at build time (lite/ and full/ configs).
  },
  define: {
    "globalThis.__VORN_RUNTIME__": JSON.stringify("dev"),
  },
})
