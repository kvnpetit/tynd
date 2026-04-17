//! Embedded SQLite via `rusqlite` (bundled). Mirrors what `bun:sqlite` gives
//! full so lite apps can use real SQL instead of only k/v `store`.

use dashmap::DashMap;
use parking_lot::Mutex;
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params_from_iter, Connection};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

type Connections = DashMap<u64, Mutex<Connection>>;

fn connections() -> &'static Connections {
    static C: OnceLock<Connections> = OnceLock::new();
    C.get_or_init(DashMap::new)
}

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "open" => open(args),
        "close" => close(args),
        "exec" => exec(args),
        "query" => query(args),
        "queryOne" => query_one(args),
        "list" => list(),
        _ => Err(format!("sql.{method}: unknown method")),
    }
}

fn open(args: &Value) -> Result<Value, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "sql.open: missing 'path' (':memory:' for in-memory)".to_string())?;

    let on_disk = path != ":memory:" && !path.is_empty();
    let conn = if on_disk {
        Connection::open(path)
    } else {
        Connection::open_in_memory()
    }
    .map_err(|e| format!("sql.open({path}): {e}"))?;

    if on_disk {
        // WAL lets readers proceed while a writer commits; synchronous=NORMAL
        // is the SQLite-recommended pairing (durable on commit, tolerant of
        // OS crash — only a power-loss before fsync can lose the last txn).
        // Errors are non-fatal: the DB is usable under default rollback mode.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let _ = conn.pragma_update(None, "synchronous", "NORMAL");
        // 64 MiB page cache (negative = size in KiB) — huge speedup for
        // repeated reads on typical app workloads while capping RAM growth.
        let _ = conn.pragma_update(None, "cache_size", -65_536);
        // Keep temp B-trees (sort/hash/group by) in RAM instead of spilling
        // to /tmp. Typical join+order_by gets 5-20x faster.
        let _ = conn.pragma_update(None, "temp_store", "MEMORY");
        // Read the DB file via mmap (256 MiB cap). Avoids a second kernel
        // buffer on top of the page cache; read-heavy queries get ~20% faster.
        let _ = conn.pragma_update(None, "mmap_size", 268_435_456_i64);
    }

    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    connections().insert(id, Mutex::new(conn));
    Ok(json!({ "id": id }))
}

fn close(args: &Value) -> Result<Value, String> {
    let id = id_arg(args, "sql.close")?;
    connections().remove(&id);
    Ok(Value::Null)
}

fn exec(args: &Value) -> Result<Value, String> {
    let id = id_arg(args, "sql.exec")?;
    let sql = sql_arg(args)?;
    let params = params_arg(args);

    let conn_cell = connections()
        .get(&id)
        .ok_or_else(|| format!("sql.exec: connection {id} not found"))?;
    let conn = conn_cell.lock();

    let changes = conn
        .execute(&sql, params_from_iter(params.iter()))
        .map_err(|e| format!("sql.exec: {e}"))?;
    let last_id = conn.last_insert_rowid();
    Ok(json!({ "changes": changes, "lastInsertId": last_id }))
}

fn query(args: &Value) -> Result<Value, String> {
    let id = id_arg(args, "sql.query")?;
    let sql = sql_arg(args)?;
    let params = params_arg(args);

    let conn_cell = connections()
        .get(&id)
        .ok_or_else(|| format!("sql.query: connection {id} not found"))?;
    let conn = conn_cell.lock();

    let mut stmt = conn.prepare(&sql).map_err(|e| format!("sql.query: {e}"))?;
    let col_names: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let rows_iter = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            Ok(row_to_json(row, &col_names))
        })
        .map_err(|e| format!("sql.query: {e}"))?;

    let mut out = Vec::new();
    for row in rows_iter {
        out.push(row.map_err(|e| format!("sql.query row: {e}"))?);
    }
    Ok(Value::Array(out))
}

fn query_one(args: &Value) -> Result<Value, String> {
    let v = query(args)?;
    match v {
        Value::Array(mut arr) if !arr.is_empty() => Ok(arr.remove(0)),
        _ => Ok(Value::Null),
    }
}

#[allow(clippy::unnecessary_wraps)] // dispatch expects Result — uniform return shape
fn list() -> Result<Value, String> {
    let ids: Vec<Value> = connections()
        .iter()
        .map(|r| Value::Number((*r.key()).into()))
        .collect();
    Ok(Value::Array(ids))
}

fn id_arg(args: &Value, ctx: &str) -> Result<u64, String> {
    args.get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{ctx}: missing 'id'"))
}

fn sql_arg(args: &Value) -> Result<String, String> {
    args.get("sql")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| "sql: missing 'sql' string".to_string())
}

fn params_arg(args: &Value) -> Vec<SqlValue> {
    args.get("params")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(json_to_sql).collect())
        .unwrap_or_default()
}

fn json_to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SqlValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                SqlValue::Real(f)
            } else {
                SqlValue::Null
            }
        },
        Value::String(s) => SqlValue::Text(s.clone()),
        Value::Array(_) | Value::Object(_) => SqlValue::Text(v.to_string()),
    }
}

fn row_to_json(row: &rusqlite::Row<'_>, cols: &[String]) -> Value {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let mut m = Map::with_capacity(cols.len());
    for (i, name) in cols.iter().enumerate() {
        let val = match row.get_ref(i) {
            Ok(ValueRef::Integer(n)) => Value::Number(n.into()),
            Ok(ValueRef::Real(f)) => {
                serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
            },
            Ok(ValueRef::Text(b)) => {
                Value::String(std::str::from_utf8(b).unwrap_or("").to_string())
            },
            // Blobs emit as base64 so the JSON payload stays text-only.
            Ok(ValueRef::Blob(b)) => Value::String(STANDARD.encode(b)),
            Ok(ValueRef::Null) | Err(_) => Value::Null,
        };
        m.insert(name.clone(), val);
    }
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_mem() -> u64 {
        let r = open(&json!({ "path": ":memory:" })).unwrap();
        r["id"].as_u64().unwrap()
    }

    #[test]
    fn exec_insert_then_query_back() {
        let id = open_mem();
        exec(&json!({ "id": id, "sql": "CREATE TABLE u(id INTEGER PRIMARY KEY, name TEXT)" }))
            .unwrap();
        let ins = exec(&json!({
            "id": id,
            "sql": "INSERT INTO u(name) VALUES (?1)",
            "params": ["Alice"]
        }))
        .unwrap();
        assert_eq!(ins["changes"].as_u64().unwrap(), 1);
        assert_eq!(ins["lastInsertId"].as_u64().unwrap(), 1);

        let rows = query(&json!({ "id": id, "sql": "SELECT id, name FROM u" })).unwrap();
        let arr = rows.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"].as_str().unwrap(), "Alice");
        assert_eq!(arr[0]["id"].as_u64().unwrap(), 1);
        close(&json!({ "id": id })).unwrap();
    }

    #[test]
    fn query_one_returns_null_on_empty() {
        let id = open_mem();
        exec(&json!({ "id": id, "sql": "CREATE TABLE t(v INTEGER)" })).unwrap();
        let r = query_one(&json!({ "id": id, "sql": "SELECT v FROM t" })).unwrap();
        assert!(r.is_null());
        close(&json!({ "id": id })).unwrap();
    }

    #[test]
    fn params_bind_multiple_types() {
        let id = open_mem();
        exec(&json!({ "id": id, "sql": "CREATE TABLE m(i INTEGER, f REAL, s TEXT, n INTEGER)" }))
            .unwrap();
        exec(&json!({
            "id": id,
            "sql": "INSERT INTO m VALUES (?1, ?2, ?3, ?4)",
            "params": [42, 2.5, "hi", null]
        }))
        .unwrap();
        let rows = query(&json!({ "id": id, "sql": "SELECT * FROM m" })).unwrap();
        let row = &rows[0];
        assert_eq!(row["i"].as_i64().unwrap(), 42);
        assert!((row["f"].as_f64().unwrap() - 2.5).abs() < 1e-9);
        assert_eq!(row["s"].as_str().unwrap(), "hi");
        assert!(row["n"].is_null());
        close(&json!({ "id": id })).unwrap();
    }

    #[test]
    fn missing_id_errors() {
        assert!(exec(&json!({ "sql": "SELECT 1" })).is_err());
        assert!(close(&json!({})).is_err());
    }

    #[test]
    fn closed_connection_is_no_longer_listed() {
        let id = open_mem();
        close(&json!({ "id": id })).unwrap();
        let ids = list().unwrap();
        let has = ids
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_u64() == Some(id));
        assert!(!has);
    }

    #[test]
    fn on_disk_open_enables_wal() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("wal.db");
        let r = open(&json!({ "path": db_path.to_string_lossy() })).unwrap();
        let id = r["id"].as_u64().unwrap();
        let mode = query_one(&json!({ "id": id, "sql": "PRAGMA journal_mode" })).unwrap();
        let jm = mode["journal_mode"]
            .as_str()
            .expect("journal_mode column present");
        assert_eq!(jm.to_lowercase(), "wal");
        close(&json!({ "id": id })).unwrap();
    }
}
