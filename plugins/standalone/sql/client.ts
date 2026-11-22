import { createPluginClient } from "@tynd/core/client";
import type sqlBackend from "./backend";
import type { SqlExecResult, SqlParam } from "./types";

const api = createPluginClient<typeof sqlBackend>("sql");

/** Open SQLite connection handle. Outlives the call site until `close()` runs. */
export interface SqlConnection {
  id: number;
  exec(sql: string, params?: SqlParam[]): Promise<SqlExecResult>;
  query<T = Record<string, unknown>>(sql: string, params?: SqlParam[]): Promise<T[]>;
  queryOne<T = Record<string, unknown>>(sql: string, params?: SqlParam[]): Promise<T | null>;
  close(): Promise<void>;
}

function makeConnection(id: number): SqlConnection {
  return {
    id,
    exec: (sql, params) => api.exec({ id, sql, params }),
    query: <T>(sql: string, params?: SqlParam[]) => api.query({ id, sql, params }) as Promise<T[]>,
    queryOne: <T>(sql: string, params?: SqlParam[]) =>
      api.queryOne({ id, sql, params }) as Promise<T | null>,
    close: () => api.close({ id }),
  };
}

/** Frontend bindings for the Bun-backed SQLite plugin. */
export const sql = {
  async open(path: string): Promise<SqlConnection> {
    const { id } = await api.open({ path });
    return makeConnection(id);
  },
  list(): Promise<number[]> {
    return api.list();
  },
};

export type { SqlExecResult, SqlParam, SqlRow } from "./types";
export default sql;
