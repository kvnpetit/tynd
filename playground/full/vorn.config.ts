import type { VornConfig } from "@vorn/cli"

// full — vorn configuration
export default {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "dist",
} satisfies VornConfig
