import { app, createEmitter } from "@vorn/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export async function greet(name: string): Promise<string> {
  return `Hello, ${name}! Welcome to react-full.`
}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({
  window: {
    title: "react-full",
    width: 1200,
    height: 800,
    center: true,
  },
})
