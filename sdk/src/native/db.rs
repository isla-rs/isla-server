//! `platform:app/db` on an in-memory SQLite, one database per thread, with
//! the host's settings and transaction rules (`docs/abi.md` § db): `begin`
//! is `BEGIN IMMEDIATE`, `begin` inside a transaction and `commit` or
//! `rollback` outside one answer `transaction`.

use std::cell::RefCell;

use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{Connection, ErrorCode, params_from_iter};

use crate::db::{Error, Rows, Value};

thread_local! {
    static CONN: RefCell<Option<Connection>> = const { RefCell::new(None) };
}

/// Runs statements (any number, `;`-separated) on this thread's database,
/// outside the app's view: a schema or rows for a test.
pub fn execute_batch(sql: &str) -> Result<(), Error> {
    with(|conn| conn.execute_batch(sql).map_err(sql_error))
}

/// Drops this thread's database; the next call opens an empty one.
pub fn reset() {
    CONN.with_borrow_mut(|conn| *conn = None);
}

pub(crate) fn query(sql: &str, params: &[Value]) -> Result<Rows, Error> {
    with(|conn| {
        let mut stmt = conn.prepare_cached(sql).map_err(sql_error)?;
        let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        let mut cursor = stmt.query(params_from_iter(params.iter().map(to_sql))).map_err(sql_error)?;
        let mut rows = Vec::new();
        while let Some(row) = cursor.next().map_err(sql_error)? {
            let cells = (0..columns.len()).map(|i| row.get_ref(i).map(from_sql)).collect::<Result<_, _>>();
            rows.push(cells.map_err(sql_error)?);
        }
        Ok(Rows { columns, rows })
    })
}

pub(crate) fn execute(sql: &str, params: &[Value]) -> Result<u64, Error> {
    with(|conn| {
        let mut stmt = conn.prepare_cached(sql).map_err(sql_error)?;
        let changed = stmt.execute(params_from_iter(params.iter().map(to_sql))).map_err(sql_error)?;
        Ok(changed as u64)
    })
}

pub(crate) fn begin() -> Result<(), Error> {
    with(|conn| {
        if !conn.is_autocommit() {
            return Err(Error::Transaction("already in a transaction".to_string()));
        }
        conn.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)
    })
}

pub(crate) fn commit() -> Result<(), Error> {
    end("COMMIT")
}

pub(crate) fn rollback() -> Result<(), Error> {
    end("ROLLBACK")
}

fn end(sql: &str) -> Result<(), Error> {
    with(|conn| {
        if conn.is_autocommit() {
            return Err(Error::Transaction("not in a transaction".to_string()));
        }
        let result = conn.execute_batch(sql).map_err(sql_error);
        // A failed commit rolls back, as on the host.
        if result.is_err() && !conn.is_autocommit() {
            let _ = conn.execute_batch("ROLLBACK");
        }
        result
    })
}

fn with<T>(f: impl FnOnce(&mut Connection) -> Result<T, Error>) -> Result<T, Error> {
    CONN.with_borrow_mut(|slot| {
        let conn = match slot {
            Some(conn) => conn,
            None => slot.insert(open().map_err(sql_error)?),
        };
        f(conn)
    })
}

/// The host's settings that matter in memory (no WAL or fsync to set).
fn open() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "trusted_schema", "OFF")?;
    Ok(conn)
}

fn to_sql(value: &Value) -> SqlValue {
    match value {
        Value::Null => SqlValue::Null,
        Value::Int(i) => SqlValue::Integer(*i),
        Value::Real(f) => SqlValue::Real(*f),
        Value::Text(s) => SqlValue::Text(s.clone()),
        Value::Blob(b) => SqlValue::Blob(b.clone()),
    }
}

fn from_sql(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Real(f),
        ValueRef::Text(bytes) => Value::Text(String::from_utf8_lossy(bytes).into_owned()),
        ValueRef::Blob(bytes) => Value::Blob(bytes.to_vec()),
    }
}

fn sql_error(e: rusqlite::Error) -> Error {
    match e.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => Error::Busy,
        _ => Error::Sql(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::db;

    fn schema() {
        super::execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL CHECK (name <> ''))").unwrap();
    }

    #[test]
    fn rows_and_changes() {
        schema();
        assert_eq!(db::execute("INSERT INTO t (name) VALUES (?), (?)", &["a".into(), "b".into()]).unwrap(), 2);
        let rows = db::query("SELECT id, name, NULL AS absent FROM t ORDER BY id", &[]).unwrap();
        assert_eq!(rows.columns, ["id", "name", "absent"]);
        assert_eq!(format!("{:?}", rows.rows[1]), r#"[Value::Int(2), Value::Text("b"), Value::Null]"#);
        let err = db::execute("INSERT INTO t (name) VALUES ('')", &[]).unwrap_err();
        assert!(matches!(&err, db::Error::Sql(m) if m.contains("CHECK constraint failed")), "{err:?}");
    }

    #[test]
    fn a_failed_transaction_rolls_back() {
        schema();
        let failed = db::transaction(|| {
            db::execute("INSERT INTO t (name) VALUES ('kept?')", &[])?;
            db::execute("INSERT INTO t (name) VALUES ('')", &[])
        });
        assert!(failed.is_err());
        assert_eq!(db::query("SELECT count(*) FROM t", &[]).unwrap().rows[0][0].as_int(), Some(0));
        assert!(matches!(db::commit(), Err(db::Error::Transaction(m)) if m == "not in a transaction"));
        db::begin().unwrap();
        assert!(matches!(db::begin(), Err(db::Error::Transaction(m)) if m == "already in a transaction"));
        db::rollback().unwrap();
    }

    #[test]
    fn each_thread_has_its_own_database() {
        schema();
        db::execute("INSERT INTO t (name) VALUES ('here')", &[]).unwrap();
        let elsewhere = std::thread::spawn(|| db::query("SELECT * FROM t", &[]).map(|_| ())).join().unwrap();
        assert!(matches!(elsewhere, Err(db::Error::Sql(m)) if m.contains("no such table")));
        super::reset();
        assert!(db::query("SELECT * FROM t", &[]).is_err(), "empty again");
    }
}
