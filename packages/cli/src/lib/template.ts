/**
 * Template strings used by `tynd init` and `tynd create`.
 *
 * Frontend scaffolding is handled by the official framework CLIs (Vite, etc.).
 * These templates only generate the tynd-specific files.
 */

export function tyndConfig(
  name: string,
  runtime: "full" | "lite",
  frontendDir = "dist",
  frontendEntry?: string,
): string {
  const entryLine = frontendEntry ? `  frontendEntry: "${frontendEntry}",\n` : ""
  return `import type { TyndConfig } from "@tynd/cli"

// ${name} — tynd configuration
export default {
  runtime:     "${runtime}",
  backend:     "backend/main.ts",
${entryLine}  frontendDir: "${frontendDir}",
} satisfies TyndConfig
`
}

export function backendMain(
  name: string,
  runtime: "full" | "lite",
  frontendDir = "/../dist",
): string {
  // Full mode: tynd-full needs frontendDir to serve static files.
  // Lite mode: frontendDir is passed via CLI --frontend-dir, not needed in code.
  const frontendLine =
    runtime === "full" ? `\n  frontendDir: import.meta.dir + "${frontendDir}",` : ""

  // Lite mode does not support async backend functions (QuickJS executes synchronously)
  const greetFn =
    runtime === "lite"
      ? `export function greet(name: string): string {
  return \`Hello, \${name}! Welcome to ${name}.\`
}`
      : `export async function greet(name: string): Promise<string> {
  return \`Hello, \${name}! Welcome to ${name}.\`
}`

  return `import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

${greetFn}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({${frontendLine}
  window: {
    title:  "${name}",
    width:  1200,
    height: 800,
    center: true,
  },
})
`
}
