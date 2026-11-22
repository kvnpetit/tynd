import { Database } from "bun:sqlite";
import { definePlugin, hostCall } from "@tynd/core";
import type {
  ConnectionArgs,
  OpenArgs,
  OpenResult,
  SqlExecResult,
  SqlParam,
  SqlPluginMethods,
  SqlRow,
  StatementArgs,
} from "./types";

const MAX_CONNECTIONS = 256;
const MAX_ROWS = 100_000;
const MAX_CELL_BYTES = 16 * 1024 * 1024;
const MAX_RESULT_BYTES = 64 * 1024 * 1024;

const connections = new Map<number, Database>();
let nextConnectionId = 0;

function allocateConnectionId(): number {
  for (let attempts = 0; attempts < Number.MAX_SAFE_INTEGER; attempts++) {
    nextConnectionId = nextConnectionId >= Number.MAX_SAFE_INTEGER ? 1 : nextConnectionId + 1;
    if (!connections.has(nextConnectionId)) return nextConnectionId;
  }
  throw new Error("sql.open: no connection id available");
}

function connection(id: number): Database {
  const db = connections.get(id);
  if (!db) throw new Error(`connection ${id} not found`);
  return db;
}

/*
 * Bun's SQLite API does not expose sqlite3_set_authorizer, so statements are
 * scanned token by token (ignoring comments and string contents) for every
 * construct that can read or write a file outside the open() path gate:
 *
 * - ATTACH (opens another database file);
 * - VACUUM [schema] INTO <file> (writes a full database copy anywhere);
 * - load_extension() (loads native code from a path);
 * - PRAGMA temp_store_directory / data_store_directory / lock_proxy_file
 *   (redirect SQLite's temporary, database and lock files).
 */

/** One lexical token of a statement; `next` is the index just past it. */
interface SqlToken {
  next: number;
  /** Lower-cased bare word. */
  word?: string;
  /** Unquoted text of a quoted or bracketed identifier, or of a string literal. */
  quoted?: string;
  /** True for a single-quoted string literal. */
  literal?: boolean;
  semicolon?: boolean;
}

function skipQuotedSql(sql: string, index: number, quote: string): number {
  let cursor = index + 1;
  while (cursor < sql.length) {
    if (sql[cursor] !== quote) {
      cursor += 1;
      continue;
    }
    if (sql[cursor + 1] === quote) {
      cursor += 2;
      continue;
    }
    return cursor + 1;
  }
  return cursor;
}

function skipBracketIdentifier(sql: string, index: number): number {
  let cursor = index + 1;
  while (cursor < sql.length && sql[cursor] !== "]") cursor += 1;
  return cursor + 1;
}

function skipLineComment(sql: string, index: number): number {
  let cursor = index + 2;
  while (cursor < sql.length && sql[cursor] !== "\n") cursor += 1;
  return cursor;
}

function skipBlockComment(sql: string, index: number): number {
  let cursor = index + 2;
  while (cursor < sql.length && !(sql[cursor] === "*" && sql[cursor + 1] === "/")) {
    cursor += 1;
  }
  return cursor + 2;
}

function readSqlWord(sql: string, index: number): SqlToken {
  let cursor = index + 1;
  while (cursor < sql.length && /[A-Za-z0-9_]/.test(sql[cursor] ?? "")) cursor += 1;
  return { next: cursor, word: sql.slice(index, cursor).toLowerCase() };
}

function readQuoted(sql: string, index: number, quote: string): SqlToken {
  const next = skipQuotedSql(sql, index, quote);
  const raw = sql.slice(index + 1, Math.max(index + 1, next - 1));
  return { next, quoted: raw.split(quote + quote).join(quote), literal: quote === "'" };
}

function scanSqlToken(sql: string, index: number): SqlToken {
  const char = sql[index];
  if (char === "'" || char === '"' || char === "`") return readQuoted(sql, index, char);
  if (char === "[") {
    const next = skipBracketIdentifier(sql, index);
    return { next, quoted: sql.slice(index + 1, Math.max(index + 1, next - 1)) };
  }
  if (char === "-" && sql[index + 1] === "-") return { next: skipLineComment(sql, index) };
  if (char === "/" && sql[index + 1] === "*") return { next: skipBlockComment(sql, index) };
  if (char === ";") return { next: index + 1, semicolon: true };
  if (char && /[A-Za-z_]/.test(char)) return readSqlWord(sql, index);
  return { next: index + 1 };
}

/**
 * Every PRAGMA that takes a filesystem path. `lock_proxy_file` (Apple builds,
 * SQLITE_ENABLE_LOCKING_STYLE) makes SQLite create a lock file and its parent
 * directories at the given path.
 */
const BLOCKED_PRAGMAS = new Set([
  "temp_store_directory",
  "data_store_directory",
  "lock_proxy_file",
]);

interface StatementState {
  sawVacuum: boolean;
  sawPragma: boolean;
}

/** Forbidden construct named by one token, updating the per-statement state. */
function checkToken(token: SqlToken, state: StatementState): string | null {
  if (token.semicolon) {
    state.sawVacuum = false;
    state.sawPragma = false;
    return null;
  }
  // Single-quoted strings only matter inside a PRAGMA statement, where
  // SQLite accepts them as the pragma name.
  if (token.literal && !state.sawPragma) return null;
  const bare = token.word;
  const name = bare ?? token.quoted?.toLowerCase();
  if (name === undefined) return null;
  if (name === "load_extension") return "load_extension";
  if (state.sawPragma && BLOCKED_PRAGMAS.has(name)) return `PRAGMA ${name}`;
  if (bare === "attach") return "ATTACH";
  if (bare === "into" && state.sawVacuum) return "VACUUM INTO";
  if (bare === "vacuum") state.sawVacuum = true;
  if (bare === "pragma") state.sawPragma = true;
  return null;
}

/**
 * Name of the first file-touching construct found in `sql`, or null. A token
 * scan, not a parser, so it errs on the side of rejecting.
 */
export function findForbiddenSql(sql: string): string | null {
  const state: StatementState = { sawVacuum: false, sawPragma: false };
  let index = 0;
  while (index < sql.length) {
    const token = scanSqlToken(sql, index);
    index = token.next;
    const found = checkToken(token, state);
    if (found) return found;
  }
  return null;
}

export function containsAttachKeyword(sql: string): boolean {
  return findForbiddenSql(sql) === "ATTACH";
}

function rejectForbidden(sql: string): void {
  const found = findForbiddenSql(sql);
  if (found === null) return;
  throw new Error(
    `sql: ${found} disabled — every database path must pass through sql.open() security checks`,
  );
}

function bindParams(params: SqlParam[] | null | undefined): SqlParam[] {
  return params ?? [];
}

class ResultBudget {
  private bytes = 0;

  add(bytes: number): void {
    this.bytes += bytes;
    if (this.bytes > MAX_RESULT_BYTES) {
      throw new Error(
        `result payload exceeds ${MAX_RESULT_BYTES}-byte cap; paginate with LIMIT/OFFSET`,
      );
    }
  }
}

function jsonCell(value: unknown, budget: ResultBudget): string | number | null {
  if (value == null) return null;
  if (value instanceof Uint8Array) {
    if (value.byteLength > MAX_CELL_BYTES) {
      throw new Error(`BLOB is ${value.byteLength} bytes; per-cell cap is ${MAX_CELL_BYTES} bytes`);
    }
    const encoded = Buffer.from(value).toString("base64");
    budget.add(Buffer.byteLength(encoded));
    return encoded;
  }
  if (typeof value === "string") {
    const bytes = Buffer.byteLength(value);
    if (bytes > MAX_CELL_BYTES) {
      throw new Error(`cell is ${bytes} bytes; per-cell cap is ${MAX_CELL_BYTES} bytes`);
    }
    budget.add(bytes);
    return value;
  }
  if (typeof value === "bigint") {
    budget.add(8);
    return Number(value);
  }
  if (typeof value === "number" || typeof value === "boolean") {
    budget.add(8);
    return typeof value === "boolean" ? Number(value) : value;
  }
  throw new Error(`unsupported SQLite result value (${typeof value})`);
}

function jsonRow(row: Record<string, unknown>, budget: ResultBudget): SqlRow {
  const result: SqlRow = Object.create(null) as SqlRow;
  for (const [name, value] of Object.entries(row)) {
    budget.add(Buffer.byteLength(name));
    result[name] = jsonCell(value, budget);
  }
  return result;
}

function withSqlContext<T>(operation: string, run: () => T): T {
  try {
    return run();
  } catch (error) {
    const message = errorMessage(error);
    if (error instanceof Error && message.startsWith(`sql.${operation}:`)) throw error;
    throw new Error(`sql.${operation}: ${message}`, { cause: error });
  }
}

function configure(db: Database, onDisk: boolean): void {
  db.run("PRAGMA busy_timeout = 30000");
  db.run("PRAGMA foreign_keys = ON");
  if (!onDisk) return;
  db.run("PRAGMA journal_mode = WAL");
  db.run("PRAGMA synchronous = NORMAL");
  db.run("PRAGMA cache_size = -65536");
  db.run("PRAGMA temp_store = MEMORY");
  db.run("PRAGMA mmap_size = 268435456");
}

const methods = {
  async open({ path }: OpenArgs): Promise<OpenResult> {
    if (connections.size >= MAX_CONNECTIONS) {
      throw new Error(`sql.open: connection limit reached (max ${MAX_CONNECTIONS})`);
    }

    const onDisk = path !== ":memory:" && path !== "";
    let databasePath = ":memory:";
    if (onDisk) {
      if (path.toLowerCase().startsWith("file:") || path.includes("?")) {
        throw new Error(
          `sql.open: SQLite URI form is not allowed (got '${path}'); pass a plain filesystem path`,
        );
      }
      databasePath = await hostCall<string>("security", "checkFs", {
        path,
        op: "readwrite",
      });
    }

    let db: Database | undefined;
    try {
      db = new Database(databasePath, { create: true, readwrite: true });
      configure(db, onDisk);
      const id = allocateConnectionId();
      connections.set(id, db);
      return { id };
    } catch (error) {
      db?.close();
      throw new Error(`sql.open(${path || ":memory:"}): ${errorMessage(error)}`);
    }
  },

  close({ id }: ConnectionArgs): void {
    const db = connections.get(id);
    if (!db) return;
    connections.delete(id);
    withSqlContext("close", () => db.close());
  },

  exec({ id, sql, params }: StatementArgs): SqlExecResult {
    rejectForbidden(sql);
    return withSqlContext("exec", () => {
      const result = connection(id).run(sql, bindParams(params));
      return {
        changes: result.changes,
        lastInsertId: Number(result.lastInsertRowid),
      };
    });
  },

  query({ id, sql, params }: StatementArgs): SqlRow[] {
    rejectForbidden(sql);
    return withSqlContext("query", () => {
      const statement = connection(id).query<Record<string, unknown>, SqlParam[]>(sql);
      const rows: SqlRow[] = [];
      const budget = new ResultBudget();
      for (const row of statement.iterate(...bindParams(params))) {
        if (rows.length >= MAX_ROWS) {
          throw new Error(`result set exceeds ${MAX_ROWS}-row cap; paginate with LIMIT/OFFSET`);
        }
        rows.push(jsonRow(row, budget));
      }
      return rows;
    });
  },

  queryOne({ id, sql, params }: StatementArgs): SqlRow | null {
    rejectForbidden(sql);
    return withSqlContext("queryOne", () => {
      const row = connection(id)
        .query<Record<string, unknown>, SqlParam[]>(sql)
        .get(...bindParams(params));
      return row ? jsonRow(row, new ResultBudget()) : null;
    });
  },

  list(): number[] {
    return [...connections.keys()].sort((a, b) => a - b);
  },
} satisfies SqlPluginMethods;

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export const sqlBackend = definePlugin({ name: "sql", methods });
export default sqlBackend;
