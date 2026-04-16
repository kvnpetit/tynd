import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export function greet(name: string): string {
  return `Hello, ${name}! Welcome to lite.`
}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({
  window: {
    title: "lite",
    width: 1200,
    height: 800,
    center: true,
  },
})
