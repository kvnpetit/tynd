// Minimal Tynd config used by the E2E smoke test. Kept inline-typed so
// the fixture does not require `@tynd/cli` to resolve in the test env —
// the smoke test invokes the bundler programmatically and only needs
// `@tynd/core` (which is a workspace package).
const config = {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "frontend",
  window: {
    title: "Tynd Smoke",
    width: 320,
    height: 200,
  },
} as const;

export default config;
