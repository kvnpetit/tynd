import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export function greet(name: string): string {
  return `Hello, ${name}! Welcome to the example app.`
}

export function increment(count: number): number {
  return count + 1
}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({
  window: {
    title: "example",
    width: 1200,
    height: 800,
    center: true,
  },
})
