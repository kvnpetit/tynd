import type { VornConfig } from "@vorn/cli"

export default {
  runtime:     "lite",
  backend:     "../core/backend/main.ts",
  frontendDir: "dist",
  devCommand:  "bunx --bun vite",
  devUrl:      "http://localhost:5174",
} satisfies VornConfig
