import type { TyndConfig } from "@tynd/cli"

// full — LLM chatbot playground. Uses the full runtime so Bun's native `fetch`
// can stream SSE directly from a local OpenAI-compatible server. The port is
// configurable in the UI (defaults to 13305). Switch to "lite" if you're fine
// hand-parsing SSE via the Tynd http API instead.
export default {
  runtime: "lite",
  backend: "backend/main.ts",
  frontendDir: "dist",
  bundle: {
    identifier: "dev.tynd.playground-full",
    displayName: "Tynd Chat",
    categories: ["Utility"],
  },
} satisfies TyndConfig
