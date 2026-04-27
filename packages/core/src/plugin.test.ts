import { describe, expect, test } from "bun:test"
import {
  definePlugin,
  getPluginMethod,
  isPlugin,
  PLUGIN_WIRE_PREFIX,
  parsePluginCall,
  registerPlugin,
} from "./plugin"

// Each test uses a unique plugin name so the shared globalThis registry
// doesn't collide across runs.
let nextId = 0
const uniqueName = () => `test-plugin-${++nextId}`

describe("definePlugin", () => {
  test("stamps identity marker + preserves fields", () => {
    const p = definePlugin({
      name: "foo",
      methods: {
        greet: (who: string) => `hi ${who}`,
      },
    })
    expect(p.__tynd_plugin__).toBe(true)
    expect(p.name).toBe("foo")
    expect(isPlugin(p)).toBe(true)
    expect(typeof p.methods.greet).toBe("function")
  })

  test("rejects empty name", () => {
    expect(() => definePlugin({ name: "", methods: {} })).toThrow()
  })
})

describe("registerPlugin", () => {
  test("populates the registry so getPluginMethod finds the handler", () => {
    const name = uniqueName()
    const p = definePlugin({
      name,
      methods: {
        add: (a: number, b: number) => a + b,
      },
    })
    registerPlugin(p)
    const handler = getPluginMethod(name, "add")
    expect(typeof handler).toBe("function")
    expect(handler?.(2, 3)).toBe(5)
  })

  test("returns null for unknown plugin / method", () => {
    expect(getPluginMethod("nope", "whatever")).toBeNull()
  })

  test("is idempotent when the same plugin is registered twice", () => {
    const name = uniqueName()
    const p = definePlugin({ name, methods: { ping: () => "pong" } })
    registerPlugin(p)
    expect(() => registerPlugin(p)).not.toThrow()
    expect(getPluginMethod(name, "ping")?.()).toBe("pong")
  })
})

describe("parsePluginCall", () => {
  test("recognizes well-formed wire names", () => {
    const parsed = parsePluginCall(`${PLUGIN_WIRE_PREFIX}terminal$spawn`)
    expect(parsed).toEqual({ plugin: "terminal", method: "spawn" })
  })

  test("rejects non-plugin fns", () => {
    expect(parsePluginCall("hello")).toBeNull()
    expect(parsePluginCall("$plugin$")).toBeNull() // empty name
    expect(parsePluginCall("$plugin$foo$")).toBeNull() // empty method
  })
})
