//! Tests without the host (feature `native`): the platform's imports run in
//! memory, one database, store and set of secrets per test thread, and the
//! app answers requests built here.
//!
//! ```ignore
//! use islas::testing::{self, Reply};
//!
//! #[test]
//! fn adds_a_note() {
//!     testing::with_migrations(concat!(env!("CARGO_MANIFEST_DIR"), "/migrations"));
//!     let app = notes::config::app();
//!     let created = testing::run_request(&app, testing::post("/api/notes").json(&json!({"text": "hi"})));
//!     assert_eq!(created.status(), 201);
//!     let notes: Vec<Note> = testing::run_request(&app, testing::get("/api/notes")).json();
//!     assert_eq!(notes.len(), 1);
//! }
//! ```
//!
//! [`native`] has the rest: rows or a schema by hand
//! (`native::db::execute_batch`), the clock KV entries and signed tokens
//! expire on (`native::advance`), secrets (`native::secrets::set`), new
//! crypto keys (`native::crypto::rotate`). [`with_static`] names the
//! folder `Response::file` reads from (the project's `static/` by
//! default), [`with_data`] the app's `/data` (a fresh temporary folder per
//! thread by default). A response that names a file gets it from
//! [`run_request`], with the headers the host adds.

use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use islas_sdk::native;

use crate::App;
use crate::web::Method;
use crate::web::files;

/// A request to send with [`run_request`].
#[derive(Debug, Clone)]
pub struct TestRequest {
    raw: islas_sdk::Request,
}

pub fn request(method: Method, path_with_query: &str) -> TestRequest {
    TestRequest { raw: islas_sdk::Request::new(method, path_with_query) }
}

pub fn get(path_with_query: &str) -> TestRequest {
    request(Method::Get, path_with_query)
}

pub fn post(path_with_query: &str) -> TestRequest {
    request(Method::Post, path_with_query)
}

pub fn put(path_with_query: &str) -> TestRequest {
    request(Method::Put, path_with_query)
}

pub fn patch(path_with_query: &str) -> TestRequest {
    request(Method::Patch, path_with_query)
}

pub fn delete(path_with_query: &str) -> TestRequest {
    request(Method::Delete, path_with_query)
}

impl TestRequest {
    pub fn header(self, name: &str, value: &str) -> Self {
        TestRequest { raw: self.raw.with_header(name, value) }
    }

    pub fn body(self, body: impl Into<Vec<u8>>) -> Self {
        TestRequest { raw: self.raw.with_body(body) }
    }

    /// `value` as a JSON body, with its `Content-Type`.
    pub fn json<T: Serialize + ?Sized>(self, value: &T) -> Self {
        let body = serde_json::to_vec(value).expect("a test body that JSON can hold");
        self.header("content-type", "application/json").body(body)
    }

    /// `value` as a form body, with its `Content-Type`.
    pub fn form<T: Serialize + ?Sized>(self, value: &T) -> Self {
        let body = serde_urlencoded::to_string(value).expect("a test body a form can hold");
        self.header("content-type", "application/x-www-form-urlencoded").body(body)
    }

    /// Adds a cookie as a browser would send it: `value` as it is (what a
    /// [`Reply::cookie`] gave, or a plain cookie's value).
    pub fn cookie(self, name: &str, value: &str) -> Self {
        self.header("cookie", &format!("{name}={value}"))
    }

    /// Adds a sealed cookie holding `value`, as if the app had set it.
    pub fn sealed_cookie(self, name: &str, value: &str) -> Self {
        let sealed = crate::crypto::seal(name, value.as_bytes()).expect("a cookie value under 4 KiB");
        self.cookie(name, &sealed)
    }
}

/// What the router answered.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

impl Reply {
    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The body as text; panics if it isn't UTF-8.
    pub fn text(&self) -> &str {
        std::str::from_utf8(&self.body).expect("a UTF-8 body")
    }

    /// The body read from JSON; panics, with the body, if it isn't that.
    pub fn json<T: DeserializeOwned>(&self) -> T {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not the JSON expected ({e}): {}", self.text()))
    }

    /// The value the reply set cookie `name` to (sealed, if the app sealed
    /// it), to send back with [`TestRequest::cookie`]; `None` when the
    /// reply didn't set it or told the browser to drop it.
    pub fn cookie(&self, name: &str) -> Option<String> {
        let header = self.set_cookie(name)?;
        let (pair, attributes) = header.split_once(';').unwrap_or((header, ""));
        let dropped = attributes.split(';').any(|a| a.trim().eq_ignore_ascii_case("max-age=0"));
        (!dropped).then(|| pair.split_once('=').map_or("", |(_, value)| value).to_string())
    }

    /// The whole `Set-Cookie` header for cookie `name`, attributes included.
    pub fn set_cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n == "set-cookie")
            .filter_map(|(_, value)| std::str::from_utf8(value).ok())
            .find(|value| value.split_once('=').is_some_and(|(n, _)| n == name))
    }
}

/// Makes `Response::file` and `Response::page` read from `dir` in this
/// thread, as they read from the version's `static/` on the host. Without
/// it, from `static/` in the current directory (the project's, under
/// `cargo test`).
pub fn with_static(dir: impl AsRef<Path>) {
    files::set_root(files::Folder::Static, dir.as_ref().to_path_buf());
}

/// Makes `dir` the app's `/data` in this thread ([`crate::data::path`],
/// `Response::data_file`). Without it, a fresh temporary folder per thread.
pub fn with_data(dir: impl AsRef<Path>) {
    files::set_root(files::Folder::Data, dir.as_ref().to_path_buf());
}

/// Sends `req` through the app's router, as the host would. A response
/// naming a file (`Response::file`, `data_file`, …) gets the file as its
/// body, as the host sends it (without `ETag` and ranges).
pub fn run_request(app: &App, req: TestRequest) -> Reply {
    let head = *req.raw.method() == Method::Head;
    let res = app.router().handle(req.raw);
    let (status, headers, body) = files::as_the_host_sends(res.status_code(), res.headers(), res.body(), head);
    Reply { status, headers, body }
}

/// Applies the app's migrations from `dir` to this thread's database, as
/// the platform does at a deploy: `<YYYYMMDDHHMMSS>_<name>.sql` in version
/// order, each in a transaction; hidden files skipped. Panics, naming the
/// file, on anything else or a failing migration.
pub fn with_migrations(dir: impl AsRef<Path>) {
    let dir = dir.as_ref();
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("migrations in {}: {e}", dir.display()));
    let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in entries {
        let path = entry.expect("reading the migrations folder").path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        let version = migration_version(&name)
            .unwrap_or_else(|| panic!("{name} in {}: not <YYYYMMDDHHMMSS>_<name>.sql", dir.display()));
        if files.iter().any(|(v, _)| *v == version) {
            panic!("two migrations with version {version} in {}", dir.display());
        }
        files.push((version, path));
    }
    files.sort();
    for (_, path) in files {
        let sql = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        if let Err(e) = native::db::execute_batch(&format!("BEGIN;\n{sql}\n;COMMIT;")) {
            let _ = native::db::execute_batch("ROLLBACK");
            panic!("migration {}: {e:?}", path.display());
        }
    }
}

/// The version of a migration file name, if it is one.
fn migration_version(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".sql")?;
    let (version, rest) = stem.split_once('_')?;
    let ok = version.len() == 14
        && version.bytes().all(|b| b.is_ascii_digit())
        && !rest.is_empty()
        && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    ok.then(|| version.to_string())
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::db;
    use crate::web::{self, Request, Response};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Note {
        id: i64,
        text: String,
    }

    fn list(_req: Request) -> web::Result {
        Ok(Response::json(&db::query_as::<Note>("SELECT id, text FROM notes ORDER BY id", &[])?))
    }

    fn add(req: Request) -> web::Result {
        #[derive(Deserialize)]
        struct New {
            text: String,
        }
        let new: New = req.json()?;
        let note: Note = db::query_one("INSERT INTO notes (text) VALUES (?) RETURNING id, text", &[new.text.into()])?;
        Ok(Response::json(&note).with_status(201))
    }

    fn app() -> App {
        App::new().routes(|r| {
            r.get("/notes", list).post("/notes", add);
        })
    }

    fn migrations(files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("islas-testing-{}", islas_sdk::random::token()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, sql) in files {
            std::fs::write(dir.join(name), sql).unwrap();
        }
        dir
    }

    #[test]
    fn a_router_against_migrations() {
        let dir = migrations(&[
            ("20261010120000_more.sql", "ALTER TABLE notes ADD COLUMN tag TEXT;"),
            ("20261009120000_init.sql", "CREATE TABLE notes (id INTEGER PRIMARY KEY, text TEXT NOT NULL);"),
            (".hidden", "not SQL"),
        ]);
        with_migrations(&dir);
        let created = run_request(&app(), post("/notes").json(&serde_json::json!({"text": "hi"})));
        assert_eq!(created.status(), 201, "{}", created.text());
        assert_eq!(created.json::<Note>(), Note { id: 1, text: "hi".into() });
        assert_eq!(run_request(&app(), get("/notes")).json::<Vec<Note>>().len(), 1);
        assert_eq!(run_request(&app(), post("/notes").body("{")).status(), 400);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[should_panic(expected = "20261009120000_bad.sql")]
    fn a_failing_migration_names_its_file() {
        let dir = migrations(&[("20261009120000_bad.sql", "CREATE TABLE")]);
        with_migrations(&dir);
    }

    #[test]
    fn files_come_back_as_the_host_sends_them() {
        let dir = std::env::temp_dir().join(format!("islas-testing-data-{}", islas_sdk::random::token()));
        std::fs::create_dir_all(dir.join("exports")).unwrap();
        std::fs::write(dir.join("exports/a.csv"), "id\n1\n").unwrap();
        with_data(&dir);
        let app = App::new().routes(|r| {
            r.get("/export", |_| Ok(Response::download("exports/a.csv", "a.csv")?.with_header("cache-control", "private")));
            r.get("/gone", |_| {
                let res = Response::data_file("exports/a.csv");
                std::fs::remove_file(crate::data::path("exports/a.csv")).unwrap();
                res
            });
        });
        let r = run_request(&app, get("/export"));
        assert_eq!((r.status(), r.text()), (200, "id\n1\n"));
        assert_eq!(r.header("content-type"), Some("text/csv; charset=utf-8"));
        assert_eq!(r.header("cache-control"), Some("private"), "the app's own");
        assert_eq!(r.header("content-security-policy"), Some("sandbox"));
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(r.header("content-disposition"), Some("attachment; filename=\"a.csv\""));
        assert_eq!(r.header("x-islas-file"), None);
        let r = run_request(&app, request(Method::Head, "/export"));
        assert_eq!((r.status(), r.body().len()), (200, 0));
        assert_eq!(run_request(&app, get("/gone")).status(), 404, "named, then gone");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_names() {
        assert_eq!(migration_version("20261009143000_init.sql").as_deref(), Some("20261009143000"));
        for bad in ["2026_init.sql", "20261009143000_Init.sql", "20261009143000_.sql", "20261009143000_init.txt"] {
            assert_eq!(migration_version(bad), None, "{bad}");
        }
    }
}
