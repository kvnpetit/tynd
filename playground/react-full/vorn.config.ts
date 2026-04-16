import type { VornConfig } from "@vorn/cli"

export default {
  runtime:     "full",
  backend:     "backend/main.ts",
  frontendDir: "dist",
} satisfies VornConfig
