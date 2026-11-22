import { describe, expect, test } from "bun:test";
import sqlBackend, { containsAttachKeyword, findForbiddenSql } from "./backend";

describe("containsAttachKeyword", () => {
  test("detects ATTACH as a token in any case", () => {
    expect(containsAttachKeyword("ATTACH DATABASE 'x.db' AS x")).toBe(true);
    expect(containsAttachKeyword("select 1; attach 'y' as y")).toBe(true);
    expect(containsAttachKeyword("  \n\tAtTaCh x")).toBe(true);
  });

  test("ignores ATTACH inside strings, identifiers and comments", () => {
    expect(containsAttachKeyword("SELECT 'attach'")).toBe(false);
    expect(containsAttachKeyword('SELECT "attach" FROM t')).toBe(false);
    expect(containsAttachKeyword("SELECT `attach`, [attach] FROM t")).toBe(false);
    expect(containsAttachKeyword("SELECT 1 -- attach\n")).toBe(false);
    expect(containsAttachKeyword("SELECT 1 /* attach */")).toBe(false);
    expect(containsAttachKeyword("SELECT 'it''s attach'")).toBe(false);
  });

  test("does not match identifiers that merely contain the word", () => {
    expect(containsAttachKeyword("SELECT attachment FROM t")).toBe(false);
    expect(containsAttachKeyword("SELECT reattach FROM t")).toBe(false);
  });

  test("still sees ATTACH after a closed comment or string", () => {
    expect(containsAttachKeyword("/* hi */ ATTACH 'a' AS a")).toBe(true);
    expect(containsAttachKeyword("SELECT 'x'; ATTACH 'a' AS a")).toBe(true);
  });
});

describe("findForbiddenSql", () => {
  test("rejects VACUUM INTO regardless of case, whitespace and comments", () => {
    const attacks = [
      "VACUUM INTO '/home/u/.bashrc'",
      "vacuum into ?",
      "VACUUM\n\t INTO 'x'",
      "VACUUM/**/INTO 'x'",
      "VACUUM -- hi\n INTO 'x'",
      "VACUUM main INTO 'x'",
      "VACUUM \"main\" INTO 'x'",
      "VACUUM [main] INTO 'x'",
      "SELECT 1; VACUUM INTO 'x'",
    ];
    for (const sql of attacks) expect(findForbiddenSql(sql)).toBe("VACUUM INTO");
  });

  test("allows a plain VACUUM and unrelated uses of INTO", () => {
    expect(findForbiddenSql("VACUUM")).toBeNull();
    expect(findForbiddenSql("VACUUM main")).toBeNull();
    expect(findForbiddenSql("VACUUM; INSERT INTO t VALUES (1)")).toBeNull();
    expect(findForbiddenSql("INSERT INTO t VALUES ('vacuum into')")).toBeNull();
    expect(findForbiddenSql("SELECT 1 /* VACUUM INTO 'x' */")).toBeNull();
  });

  test("rejects load_extension, bare or quoted", () => {
    expect(findForbiddenSql("SELECT load_extension('x')")).toBe("load_extension");
    expect(findForbiddenSql("SELECT \"LOAD_EXTENSION\"('x')")).toBe("load_extension");
    expect(findForbiddenSql("SELECT 'load_extension'")).toBeNull();
  });

  test("rejects pragmas that redirect SQLite files", () => {
    expect(findForbiddenSql("PRAGMA temp_store_directory = '/tmp'")).toBe(
      "PRAGMA temp_store_directory",
    );
    expect(findForbiddenSql("pragma main.data_store_directory='C:\\\\x'")).toBe(
      "PRAGMA data_store_directory",
    );
    expect(findForbiddenSql("PRAGMA 'temp_store_directory' = 'x'")).toBe(
      "PRAGMA temp_store_directory",
    );
    expect(findForbiddenSql("PRAGMA lock_proxy_file='/Users/u/x'")).toBe("PRAGMA lock_proxy_file");
    expect(findForbiddenSql("pragma main.[Lock_Proxy_File] = ':auto:'")).toBe(
      "PRAGMA lock_proxy_file",
    );
    expect(findForbiddenSql("PRAGMA foreign_keys = ON")).toBeNull();
    expect(findForbiddenSql("PRAGMA user_version; SELECT 'temp_store_directory'")).toBeNull();
  });
});

// The default export is a plugin definition whose methods are what IPC calls.
const { methods } = sqlBackend as unknown as {
  methods: {
    open(args: { path: string }): Promise<{ id: number }>;
    close(args: { id: number }): void;
    exec(args: { id: number; sql: string; params?: unknown[] | null }): {
      changes: number;
      lastInsertId: number;
    };
    query(args: {
      id: number;
      sql: string;
      params?: unknown[] | null;
    }): Array<Record<string, unknown>>;
    queryOne(args: {
      id: number;
      sql: string;
      params?: unknown[] | null;
    }): Record<string, unknown> | null;
    list(): number[];
  };
};

describe("in-memory connection", () => {
  test("exec / query / queryOne round-trip with bound params and BLOB base64", async () => {
    const { id } = await methods.open({ path: ":memory:" });
    try {
      methods.exec({ id, sql: "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, data BLOB)" });
      const inserted = methods.exec({
        id,
        sql: "INSERT INTO t (name, data) VALUES (?, ?)",
        params: ["alice", null],
      });
      expect(inserted).toEqual({ changes: 1, lastInsertId: 1 });
      methods.exec({ id, sql: "INSERT INTO t (name, data) VALUES ('bob', x'0102')" });

      const rows = methods.query({ id, sql: "SELECT name, data FROM t ORDER BY id" });
      expect(rows).toEqual([
        { name: "alice", data: null },
        { name: "bob", data: "AQI=" },
      ]);
      expect(methods.queryOne({ id, sql: "SELECT name FROM t WHERE id = ?", params: [2] })).toEqual(
        {
          name: "bob",
        },
      );
      expect(
        methods.queryOne({ id, sql: "SELECT name FROM t WHERE id = ?", params: [99] }),
      ).toBeNull();
      expect(methods.list()).toContain(id);
    } finally {
      methods.close({ id });
    }
    expect(methods.list()).not.toContain(id);
    // Closing twice is a no-op.
    expect(() => methods.close({ id })).not.toThrow();
  });
});

describe("connection guards", () => {
  test("ATTACH is rejected on exec, query and queryOne", async () => {
    const { id } = await methods.open({ path: ":memory:" });
    try {
      for (const call of [methods.exec, methods.query, methods.queryOne]) {
        expect(() => call({ id, sql: "ATTACH DATABASE ':memory:' AS other" })).toThrow("ATTACH");
      }
    } finally {
      methods.close({ id });
    }
  });

  test("VACUUM INTO is rejected on exec, query and queryOne", async () => {
    const { id } = await methods.open({ path: ":memory:" });
    try {
      for (const call of [methods.exec, methods.query, methods.queryOne]) {
        expect(() => call({ id, sql: "vacuum /* x */ into '/tmp/out.db'" })).toThrow("VACUUM INTO");
      }
    } finally {
      methods.close({ id });
    }
  });

  test("errors carry the operation prefix and unknown ids are reported", async () => {
    const { id } = await methods.open({ path: ":memory:" });
    try {
      expect(() => methods.exec({ id, sql: "NOT SQL" })).toThrow("sql.exec:");
      expect(() => methods.query({ id: 987_654, sql: "SELECT 1" })).toThrow("not found");
    } finally {
      methods.close({ id });
    }
  });

  test("URI-form paths are refused before touching the host", async () => {
    await expect(methods.open({ path: "file:foo.db?mode=ro" })).rejects.toThrow("URI form");
  });
});
