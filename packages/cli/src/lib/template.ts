/**
 * Template strings used by `vorn init` and `vorn create`.
 *
 * Frontend scaffolding is handled by the official framework CLIs (Vite, etc.).
 * These templates only generate the vorn-specific files.
 */

export function vornConfig(
  name: string,
  runtime: "full" | "lite",
  frontendDir = "dist",
  frontendEntry?: string,
): string {
  const entryLine = frontendEntry ? `  frontendEntry: "${frontendEntry}",\n` : ""
  return `import type { VornConfig } from "@vorn/cli"

// ${name} — vorn configuration
export default {
  runtime:     "${runtime}",
  backend:     "backend/main.ts",
${entryLine}  frontendDir: "${frontendDir}",
} satisfies VornConfig
`
}

export function backendMain(
  name: string,
  runtime: "full" | "lite",
  frontendDir = "/../dist",
): string {
  // Full mode: vorn-full needs frontendDir to serve static files.
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

  return `import { app, createEmitter } from "@vorn/core"

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
