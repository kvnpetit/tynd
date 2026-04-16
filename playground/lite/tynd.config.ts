import type { TyndConfig } from "@tynd/cli"

// lite — tynd configuration
export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
} satisfies TyndConfig
