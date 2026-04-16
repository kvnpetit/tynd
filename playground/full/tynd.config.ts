import type { TyndConfig } from "@tynd/cli"

// full — tynd configuration
export default {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "dist",
} satisfies TyndConfig
