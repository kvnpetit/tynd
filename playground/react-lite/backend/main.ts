import { app, createEmitter } from "@vorn/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export function greet(name: string): string {
  return `Hello, ${name}! Welcome to react-lite.`
}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({
  window: {
    title: "react-lite",
    width: 1200,
    height: 800,
    center: true,
  },
})
