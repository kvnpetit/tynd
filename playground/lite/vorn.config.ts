import type { VornConfig } from "@vorn/cli"

// lite — vorn configuration
export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
} satisfies VornConfig
