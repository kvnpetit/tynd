import { osCall } from "./_internal.ts"

export type SqlParam = string | number | boolean | null | unknown[] | Record<string, unknown>

export interface SqlExecResult {
  changes: number
  lastInsertId: number
}

export interface SqlConnection {
  id: number
  exec(sql: string, params?: SqlParam[]): Promise<SqlExecResult>
  query<T = Record<string, unknown>>(sql: string, params?: SqlParam[]): Promise<T[]>
  queryOne<T = Record<string, unknown>>(sql: string, params?: SqlParam[]): Promise<T | null>
  close(): Promise<void>
}

function makeConn(id: number): SqlConnection {
  return {
    id,
    exec(sql, params) {
      return osCall("sql", "exec", { id, sql, params: params ?? [] })
    },
    query<T>(sql: string, params?: SqlParam[]) {
      return osCall<T[]>("sql", "query", { id, sql, params: params ?? [] })
    },
    queryOne<T>(sql: string, params?: SqlParam[]) {
      return osCall<T | null>("sql", "queryOne", { id, sql, params: params ?? [] })
    },
    close() {
      return osCall("sql", "close", { id })
    },
  }
}

export const sql = {
  async open(path: string): Promise<SqlConnection> {
    const { id } = await osCall<{ id: number }>("sql", "open", { path })
    return makeConn(id)
  },
  list(): Promise<number[]> {
    return osCall("sql", "list")
  },
}
