import { app, createEmitter } from "@vorn/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export async function greet(name: string): Promise<string> {
  return `Hello, ${name}! Welcome to full.`
}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({
  frontendDir: import.meta.dir + "/../dist",
  window: {
    title:  "full",
    width:  1200,
    height: 800,
    center: true,
  },
})
