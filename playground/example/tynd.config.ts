import type { TyndConfig } from "@tynd/cli"

// example — minimal Tynd playground for the lite runtime.
export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
  bundle: {
    identifier: "dev.tynd.playground-example",
    categories: ["Utility"],
  },
} satisfies TyndConfig
