//! The app's own SQLite database: the SDK's [`query`], [`execute`] and
//! [`transaction`], plus rows read into your types by column name.
//!
//! ```ignore
//! #[derive(Deserialize)]
//! struct Note { id: i64, text: String, done: bool, tag: Option<String> }
//!
//! let notes: Vec<Note> = db::query_as("SELECT id, text, done, tag FROM notes", &[])?;
//! let note: Option<Note> = db::query_optional("SELECT … WHERE id = ?", &[id.into()])?;
//! let count: i64 = db::query_one("SELECT count(*) FROM notes", &[])?;
//! let pair: (i64, String) = db::query_one("SELECT id, text FROM notes LIMIT 1", &[])?;
//! ```
//!
//! A struct reads columns by name (extra columns are ignored, a missing one
//! is an error unless the field is an `Option`); a tuple reads them in
//! order; a single value reads a one-column row. Cells: integers into any
//! integer type that holds them (and `bool` from `0`/`1`), reals (or
//! integers) into floats, text into `String` or a unit enum variant by
//! name, blobs into `Vec<u8>`, `NULL` into `None`. A row that doesn't fit
//! is a [`Error::Sql`] naming the column: the query and the type disagree,
//! which is the app's bug.

use serde::de::DeserializeOwned;

pub use islas_sdk::db::*;

mod row;

/// Every row of `sql`, each read into a `T`.
pub fn query_as<T: DeserializeOwned>(sql: &str, params: &[Value]) -> Result<Vec<T>, Error> {
    let rows = query(sql, params)?;
    rows.rows.iter().map(|row| row::read(&rows.columns, row)).collect()
}

/// The first row of `sql` as a `T`, or `None` when there is none.
pub fn query_optional<T: DeserializeOwned>(sql: &str, params: &[Value]) -> Result<Option<T>, Error> {
    let rows = query(sql, params)?;
    rows.rows.first().map(|row| row::read(&rows.columns, row)).transpose()
}

/// The first row of `sql` as a `T`, for a query that always has one
/// (`count(*)`, `INSERT … RETURNING`); none is an [`Error::Sql`]. For a row
/// that may be missing, use [`query_optional`].
pub fn query_one<T: DeserializeOwned>(sql: &str, params: &[Value]) -> Result<T, Error> {
    query_optional(sql, params)?.ok_or_else(|| Error::Sql("query_one: the query returned no row".to_string()))
}

#[cfg(test)]
mod tests {
    use islas_sdk::native;
    use serde::Deserialize;

    use super::*;

    fn schema() {
        native::db::execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, done INTEGER NOT NULL,
                             tag TEXT, data BLOB, status TEXT NOT NULL);
             INSERT INTO t VALUES (1, 'a', 1.5, 1, 'x', X'0102', 'open'), (2, 'b', NULL, 0, NULL, NULL, 'done');",
        )
        .unwrap();
    }

    #[derive(Deserialize, Debug, PartialEq)]
    #[serde(rename_all = "lowercase")]
    enum Status {
        Open,
        Done,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Row {
        id: u32,
        name: String,
        score: Option<f64>,
        done: bool,
        tag: Option<String>,
        data: Option<Vec<u8>>,
        status: Status,
    }

    #[test]
    fn every_column_type() {
        schema();
        let rows: Vec<Row> = query_as("SELECT * FROM t ORDER BY id", &[]).unwrap();
        assert_eq!(
            rows,
            [
                Row {
                    id: 1,
                    name: "a".into(),
                    score: Some(1.5),
                    done: true,
                    tag: Some("x".into()),
                    data: Some(vec![1, 2]),
                    status: Status::Open
                },
                Row { id: 2, name: "b".into(), score: None, done: false, tag: None, data: None, status: Status::Done },
            ]
        );
    }

    #[test]
    fn tuples_single_values_and_missing_rows() {
        schema();
        let count: i64 = query_one("SELECT count(*) FROM t", &[]).unwrap();
        assert_eq!(count, 2);
        let pair: (i64, String) = query_one("SELECT id, name FROM t WHERE id = ?", &[2.into()]).unwrap();
        assert_eq!(pair, (2, "b".into()));
        let real: f64 = query_one("SELECT 3", &[]).unwrap();
        assert_eq!(real, 3.0, "an integer reads as a float");
        assert_eq!(query_optional::<i64>("SELECT id FROM t WHERE id = 9", &[]).unwrap(), None);
        assert!(matches!(query_one::<i64>("SELECT id FROM t WHERE id = 9", &[]), Err(Error::Sql(m)) if m.contains("no row")));
    }

    #[test]
    fn a_row_that_doesnt_fit_names_the_column() {
        schema();
        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct Wrong {
            id: i64,
            name: i64,
        }
        let err = query_as::<Wrong>("SELECT id, name FROM t", &[]).unwrap_err();
        assert!(matches!(&err, Error::Sql(m) if m.contains("column `name`")), "{err:?}");
        #[derive(Deserialize, Debug)]
        #[allow(dead_code)]
        struct Missing {
            nothing: i64,
        }
        let err = query_as::<Missing>("SELECT id FROM t", &[]).unwrap_err();
        assert!(matches!(&err, Error::Sql(m) if m.contains("missing field `nothing`")), "{err:?}");
        let err = query_one::<u8>("SELECT 300", &[]).unwrap_err();
        assert!(matches!(&err, Error::Sql(_)), "out of range: {err:?}");
        let err = query_one::<bool>("SELECT 2", &[]).unwrap_err();
        assert!(matches!(&err, Error::Sql(_)), "a bool is 0 or 1: {err:?}");
    }
}
