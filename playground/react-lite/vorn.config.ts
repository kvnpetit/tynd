import type { VornConfig } from "@vorn/cli"

export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
} satisfies VornConfig
