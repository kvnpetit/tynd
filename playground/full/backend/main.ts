import { app, createEmitter } from "@tynd/core"

export const events = createEmitter<{
  ready: { message: string }
}>()

export async function greet(name: string): Promise<string> {
  return `Hello, ${name}! Welcome to full.`
}

export async function increment(count: number): Promise<number> {
  return count + 1
}

app.onReady(() => {
  events.emit("ready", { message: "App loaded!" })
})

app.start({
  frontendDir: `${import.meta.dir}/../dist`,
  window: {
    title: "full",
    width: 1200,
    height: 800,
    center: true,
  },
})
