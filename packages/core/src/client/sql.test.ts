import { beforeEach, describe, expect, test } from "bun:test"
import { sql } from "./sql.ts"

type Call = { api: string; method: string; args: unknown }

function mountShim(handler: (c: Call) => unknown): Call[] {
  const calls: Call[] = []
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        const call = { api, method, args }
        calls.push(call)
        return Promise.resolve(handler(call))
      },
      os_on() {
        return () => undefined
      },
      call() {
        return Promise.resolve(null)
      },
      on() {
        return () => undefined
      },
      off() {
        return undefined
      },
    },
  }
  return calls
}

describe("sql client", () => {
  let calls: Call[]
  beforeEach(() => {
    calls = mountShim((c) => {
      if (c.method === "open") return { id: 42 }
      if (c.method === "exec") return { changes: 1, lastInsertId: 7 }
      if (c.method === "query") return [{ id: 1, name: "Alice" }]
      if (c.method === "queryOne") return { id: 1, name: "Alice" }
      if (c.method === "list") return [42]
      return null
    })
  })

  test("open returns a connection bound to the assigned id", async () => {
    const db = await sql.open(":memory:")
    expect(db.id).toBe(42)
    expect(calls[0]).toMatchObject({ api: "sql", method: "open", args: { path: ":memory:" } })
  })

  test("exec forwards sql + params and carries id through", async () => {
    const db = await sql.open(":memory:")
    const res = await db.exec("INSERT INTO t VALUES (?1)", ["x"])
    expect(res).toEqual({ changes: 1, lastInsertId: 7 })
    const execCall = calls.find((c) => c.method === "exec")!
    expect(execCall.args).toEqual({
      id: 42,
      sql: "INSERT INTO t VALUES (?1)",
      params: ["x"],
    })
  })

  test("query defaults params to []", async () => {
    const db = await sql.open(":memory:")
    await db.query("SELECT * FROM t")
    const queryCall = calls.find((c) => c.method === "query")!
    expect((queryCall.args as { params: unknown[] }).params).toEqual([])
  })

  test("queryOne returns null-or-row shape unchanged", async () => {
    const db = await sql.open(":memory:")
    const row = await db.queryOne<{ id: number; name: string }>(
      "SELECT id, name FROM t WHERE id = ?1",
      [1],
    )
    expect(row).toEqual({ id: 1, name: "Alice" })
  })

  test("close forwards the id only", async () => {
    const db = await sql.open(":memory:")
    await db.close()
    expect(calls.find((c) => c.method === "close")!.args).toEqual({ id: 42 })
  })

  test("list returns the id array", async () => {
    const ids = await sql.list()
    expect(ids).toEqual([42])
  })
})
