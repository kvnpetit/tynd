# @tynd/plugin-sql

Bundled SQLite for Tynd apps via Bun's built-in [`bun:sqlite`][bun-sqlite].
On-disk databases open with WAL + tuned pragmas
(cache size, mmap, temp_store) for solid out-of-the-box performance.

No custom Rust host or system SQLite dependency is required. Database work
runs in the shipped Bun backend; filesystem paths still pass through Tynd's
native security policy before open.

## Install

```sh
tynd add sql
```

`tynd add` installs the package and lists it in `plugins` in `tynd.config.ts`:

```ts
// tynd.config.ts
export default {
  plugins: ["@tynd/plugin-sql"],
  backend: "backend/main.ts",
  frontendDir: "dist",
}
```

```ts
// backend/main.ts
import { app } from "@tynd/core"

app.start({ window: { title: "My App" } })
```

## Use

```ts
import { sql } from "@tynd/plugin-sql"

const db = await sql.open("/path/to/app.db")  // or ":memory:"

await db.exec("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)")
const { lastInsertId } = await db.exec(
  "INSERT INTO users (name) VALUES (?1)",
  ["Daisy"],
)

const rows = await db.query<{ id: number; name: string }>(
  "SELECT id, name FROM users WHERE name LIKE ?1",
  ["%ai%"],
)

await db.close()
```

## API

| Method | Purpose |
|---|---|
| `sql.open(path)` | Open a connection — `":memory:"` or filesystem path. Returns `SqlConnection`. |
| `sql.list()` | IDs of currently open connections. |
| `conn.exec(sql, params?)` | Execute a non-query. Returns `{ changes, lastInsertId }`. |
| `conn.query<T>(sql, params?)` | Return all rows as objects. |
| `conn.queryOne<T>(sql, params?)` | Return first row or `null`. |
| `conn.close()` | Close the connection. |

Parameters use `?1`, `?2`, … placeholders (SQLite's default). JSON
values map to SQLite types: numbers → INTEGER/REAL, strings → TEXT,
arrays/objects → JSON-encoded TEXT, `null` → NULL, booleans → INTEGER
(0/1). Blobs on the read side come back as base64-encoded strings so
the JSON payload stays text-only.

## Limits

| Constraint | Value |
| ---------- | ----- |
| Max concurrent connections | 256 |
| Max rows per `query` / `queryOne` | 100 000 |
| `busy_timeout` (lock contention) | 30 s |

The connection cap and row cap throw on overflow — paginate large
result sets with `LIMIT`/`OFFSET`. `busy_timeout` is set per-connection
post-open and can be overridden with `PRAGMA busy_timeout = N`.

## Why not core?

SQLite is a whole relational engine, not an OS primitive. Apps that
don't need a database shouldn't ship its backend adapter. Keeping it
outside `@tynd/core` keeps the runtime surface slim and
makes the "API = OS primitive, plugin = built on top" rule honest.

[bun-sqlite]: https://bun.sh/docs/api/sqlite

---

See also: [tynd-apps/plugins](../README.md) — plugin index.
