/** Scalar values accepted by SQLite bind parameters over Tynd IPC. */
export type SqlParam = string | number | boolean | null;

/** JSON-safe row returned to the WebView. SQLite BLOBs are base64 strings. */
export type SqlRow = Record<string, string | number | null>;

export interface OpenArgs {
  path: string;
}

export interface OpenResult {
  id: number;
}

export interface ConnectionArgs {
  id: number;
}

export interface StatementArgs extends ConnectionArgs {
  sql: string;
  params?: SqlParam[] | null;
}

export interface SqlExecResult {
  changes: number;
  lastInsertId: number;
}

export interface SqlPluginMethods {
  open(args: OpenArgs): Promise<OpenResult>;
  close(args: ConnectionArgs): void;
  exec(args: StatementArgs): SqlExecResult;
  query(args: StatementArgs): SqlRow[];
  queryOne(args: StatementArgs): SqlRow | null;
  list(): number[];
}
