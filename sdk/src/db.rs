//! The app's own SQLite database (`platform:app/db`).
//!
//! ```ignore
//! use islas_sdk::db;
//!
//! db::execute("INSERT INTO notes (text) VALUES (?)", &["hello".into()])?;
//! let rows = db::query("SELECT id, text FROM notes WHERE id > ?", &[0.into()])?;
//! db::transaction(|| {
//!     db::execute("UPDATE accounts SET balance = balance - ? WHERE name = ?", &[30.into(), "alice".into()])?;
//!     db::execute("UPDATE accounts SET balance = balance + ? WHERE name = ?", &[30.into(), "bob".into()])?;
//!     Ok::<_, db::Error>(())
//! })?;
//! ```
//!
//! Transactions are [`transaction`] (or [`begin`], [`commit`],
//! [`rollback`]), never `BEGIN` in SQL. One left open when the event ends is
//! rolled back.

use crate::bindings::platform::app::db as host;
use crate::off_host;

pub use host::{Error, Rows, Value};

/// Runs a statement and returns its rows.
pub fn query(sql: &str, params: &[Value]) -> Result<Rows, Error> {
    if cfg!(target_arch = "wasm32") { host::query(sql, params) } else { off_host::db::query(sql, params) }
}

/// Runs a statement and returns the number of rows changed.
pub fn execute(sql: &str, params: &[Value]) -> Result<u64, Error> {
    if cfg!(target_arch = "wasm32") { host::execute(sql, params) } else { off_host::db::execute(sql, params) }
}

/// Runs `f` in a transaction: committed if it returns `Ok`, rolled back if
/// it returns `Err`. The error type takes a [`Error`] through `From`, so `?`
/// works on database calls inside.
pub fn transaction<T, E: From<Error>>(f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    begin()?;
    match f() {
        Ok(value) => {
            commit()?;
            Ok(value)
        }
        Err(e) => {
            // The error from `f` is the one that matters; a failed rollback
            // still ends the transaction (the host drops it with the event).
            let _ = rollback();
            Err(e)
        }
    }
}

/// Starts a transaction bound to this event; rolled back at the event's end
/// unless committed. Inside one, `begin` answers [`Error::Transaction`].
pub fn begin() -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") { host::begin() } else { off_host::db::begin() }
}

/// Commits the transaction. If the commit fails, the transaction is rolled
/// back.
pub fn commit() -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") { host::commit() } else { off_host::db::commit() }
}

pub fn rollback() -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") { host::rollback() } else { off_host::db::rollback() }
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// A real, or an integer as a real.
    pub fn as_real(&self) -> Option<f64> {
        match self {
            Value::Real(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_blob(&self) -> Option<&[u8]> {
        match self {
            Value::Blob(b) => Some(b),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

impl From<&String> for Value {
    fn from(s: &String) -> Self {
        Value::Text(s.clone())
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}

impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::Int(i.into())
    }
}

impl From<u32> for Value {
    fn from(i: u32) -> Self {
        Value::Int(i.into())
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Real(f)
    }
}

/// SQLite has no booleans: `1` and `0`.
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Int(b.into())
    }
}

impl From<Vec<u8>> for Value {
    fn from(b: Vec<u8>) -> Self {
        Value::Blob(b)
    }
}

impl From<&[u8]> for Value {
    fn from(b: &[u8]) -> Self {
        Value::Blob(b.to_vec())
    }
}

/// `None` is `NULL`.
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(value: Option<T>) -> Self {
        value.map_or(Value::Null, Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_from_rust_types() {
        let values: Vec<Value> = vec![
            "a".into(),
            String::from("b").into(),
            7i64.into(),
            7i32.into(),
            1.5.into(),
            true.into(),
            vec![1u8, 2].into(),
            None::<i64>.into(),
            Some("c").into(),
        ];
        assert_eq!(
            format!("{values:?}"),
            r#"[Value::Text("a"), Value::Text("b"), Value::Int(7), Value::Int(7), Value::Real(1.5), Value::Int(1), Value::Blob([1, 2]), Value::Null, Value::Text("c")]"#
        );
        assert_eq!(values[2].as_int(), Some(7));
        assert_eq!(values[0].as_text(), Some("a"));
        assert!(values[7].is_null());
    }

    #[test]
    #[cfg(not(feature = "native"))]
    fn natively_the_database_is_unavailable() {
        let err = transaction(|| Ok::<_, Error>(())).unwrap_err();
        assert!(matches!(err, Error::Unavailable(_)));
    }
}
