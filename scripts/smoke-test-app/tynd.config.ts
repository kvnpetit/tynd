// Wave 4.4 smoke-test app config.
// `runtime` is overridden by whichever host binary launches us (tynd-full
// vs tynd-lite) — the field exists for `tynd build` consumption when the
// CLI is wired in (see W4.4 follow-up).

interface SmokeTyndConfig {
  runtime: "full" | "lite";
  backend: string;
  frontendDir: string;
}

const config: SmokeTyndConfig = {
  runtime: "full",
  backend: "backend/main.ts",
  frontendDir: "src",
};

export default config;
