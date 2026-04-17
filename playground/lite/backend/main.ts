import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export function greet(name: string): string {
  return `Hello, ${name}! Welcome to lite.`
}

export function increment(count: number): number {
  return count + 1
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
