//! The response a controller returns.

use serde::Serialize;

use super::{Cookie, Error, files};

/// An outgoing response with its whole body.
#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

impl Response {
    /// 200 with `value` as JSON. A value JSON can't hold (a map whose keys
    /// aren't strings) is a 500, logged.
    pub fn json<T: Serialize + ?Sized>(value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Response::status(200).with_header("content-type", "application/json").with_body(body),
            Err(e) => {
                eprintln!("islas: a response that isn't JSON: {e}");
                Response::status(500)
                    .with_header("content-type", "application/json")
                    .with_body(r#"{"error":"internal error"}"#)
            }
        }
    }

    /// 200 with a `text/plain` body.
    pub fn text(body: impl Into<String>) -> Self {
        Response::status(200).with_header("content-type", "text/plain; charset=utf-8").with_body(body.into())
    }

    /// 200 with a `text/html` body.
    pub fn html(body: impl Into<String>) -> Self {
        Response::status(200).with_header("content-type", "text/html; charset=utf-8").with_body(body.into())
    }

    /// An empty response with this status: `Response::status(204)`.
    pub fn status(status: u16) -> Self {
        Response { status, headers: Vec::new(), body: Vec::new() }
    }

    /// 303 See Other to `location`: the browser follows it with a `GET`.
    pub fn redirect(location: &str) -> Self {
        Response::status(303).with_header("location", location)
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Adds a header (a second one with the same name adds a value).
    pub fn with_header(mut self, name: &str, value: impl Into<Vec<u8>>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// Replaces a header's values with one.
    fn set_header(mut self, name: &str, value: impl Into<Vec<u8>>) -> Self {
        self.headers.retain(|(n, _)| n != name);
        self.with_header(name, value)
    }

    /// A file of the version's `static/` folder, by its path in it: 200
    /// naming the file, which the host sends (`X-Islas-File`), with its type
    /// by extension, `Cache-Control: no-cache` and `nosniff` unless the
    /// response sets them; a 404 for a missing file, a folder, or a path
    /// that isn't relative and inside (`..`, `/etc/x`). For a page behind a
    /// check; the host serves the public ones itself. The status and the
    /// headers added after stay.
    pub fn file(path: &str) -> Result<Self, Error> {
        Ok(Response::status(200).with_header(files::HEADER, files::name(files::Folder::Static, path)?))
    }

    /// [`Response::file`] for a page: the same, always as `text/html`.
    pub fn page(path: &str) -> Result<Self, Error> {
        Ok(Response::file(path)?.set_header("content-type", "text/html; charset=utf-8"))
    }

    /// [`Response::file`] for a file under the app's `/data` (written at
    /// [`crate::data::path`]): the host adds `Content-Security-Policy:
    /// sandbox` unless the response sets a policy, as the file may be a
    /// user's (an uploaded page can't run as the app).
    pub fn data_file(path: &str) -> Result<Self, Error> {
        Ok(Response::status(200).with_header(files::HEADER, files::name(files::Folder::Data, path)?))
    }

    /// [`Response::data_file`] as a download saved as `name`.
    pub fn download(path: &str, name: &str) -> Result<Self, Error> {
        Ok(Response::data_file(path)?.attachment(name))
    }

    /// Makes the browser save the body (or the file) as `name` rather than
    /// show it: `Content-Disposition: attachment`.
    pub fn attachment(self, name: &str) -> Self {
        self.set_header("content-disposition", files::attachment(name))
    }

    /// Sets a sealed cookie: the browser carries the value but can't read or
    /// change it, and [`Request::cookie`](super::Request::cookie) opens it.
    /// A value over 4 KiB can't be sealed: the response becomes a 500,
    /// logged (as a response that isn't JSON does).
    pub fn cookie(self, cookie: Cookie) -> Self {
        match cookie.sealed_header() {
            Ok(header) => self.with_header("set-cookie", header),
            Err(e) => {
                eprintln!("islas: cookie {}: {e}", cookie.name());
                Response::status(500).with_header("content-type", "application/json").with_body(r#"{"error":"internal error"}"#)
            }
        }
    }

    /// Sets a plain (unsealed) cookie.
    pub fn plain_cookie(self, cookie: Cookie) -> Self {
        let header = cookie.header();
        self.with_header("set-cookie", header)
    }

    /// Tells the browser to drop cookie `name` (set at `Path=/`).
    pub fn remove_cookie(self, name: &str) -> Self {
        self.with_header("set-cookie", format!("{name}=; Path=/; Max-Age=0"))
    }

    pub fn status_code(&self) -> u16 {
        self.status
    }

    /// The first value of a header, by case-insensitive name, if it is UTF-8.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
    }

    pub fn headers(&self) -> &[(String, Vec<u8>)] {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Drops the body, for a `HEAD` answered by a `GET` route.
    pub(crate) fn without_body(mut self) -> Self {
        self.body.clear();
        self
    }
}

impl From<Response> for islas_sdk::Response {
    fn from(res: Response) -> Self {
        let mut out = islas_sdk::Response::new(res.status).with_body(res.body);
        for (name, value) in res.headers {
            out = out.with_header(&name, value);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders() {
        let res = Response::json(&serde_json::json!({"ok": true})).with_status(201);
        assert_eq!((res.status_code(), res.body()), (201, &br#"{"ok":true}"#[..]));
        assert_eq!(res.header("Content-Type"), Some("application/json"));
        assert_eq!(Response::redirect("/login").header("location"), Some("/login"));
        let res = Response::status(204).plain_cookie(Cookie::new("a", "1")).remove_cookie("b");
        let cookies: Vec<_> = res.headers().iter().filter(|(n, _)| n == "set-cookie").collect();
        assert_eq!(cookies.len(), 2);
    }

    #[test]
    fn a_map_with_other_keys_is_a_500() {
        let map: std::collections::BTreeMap<(u8, u8), u8> = [((1, 2), 3)].into();
        assert_eq!(Response::json(&map).status_code(), 500);
    }
}

#[cfg(test)]
mod sealed_tests {
    use super::*;
    use crate::testing::native;
    use crate::web::Request;

    fn request_with(cookies: &str) -> Request {
        Request::new(islas_sdk::Request::new(islas_sdk::Method::Get, "/").with_header("cookie", cookies), Vec::new())
    }

    #[test]
    fn sealed_cookies_round_trip_and_open_only_as_set() {
        native::reset();
        let res = Response::status(204).cookie(Cookie::new("theme", "dark; mode").max_age(std::time::Duration::from_secs(5)));
        let header = res.header("set-cookie").unwrap();
        let (pair, rest) = header.split_once(';').unwrap();
        let sealed = pair.strip_prefix("theme=").unwrap();
        assert!(!sealed.contains("dark") && !sealed.contains('%'), "{sealed}");
        assert_eq!(rest, " Path=/; Max-Age=5; HttpOnly; Secure; SameSite=Lax");

        assert_eq!(request_with(&format!("theme={sealed}")).cookie("theme").as_deref(), Some("dark; mode"));
        assert_eq!(request_with(&format!("other={sealed}")).cookie("other"), None, "moved to another name");
        assert_eq!(request_with(&format!("theme={sealed}x")).cookie("theme"), None, "changed");
        assert_eq!(request_with("theme=dark").cookie("theme"), None, "a plain value");
        assert_eq!(request_with("theme=dark").cookie_plain("theme").as_deref(), Some("dark"));
        assert_eq!(request_with(&format!("theme={sealed}")).cookie("missing"), None);
        native::crypto::rotate();
        assert_eq!(request_with(&format!("theme={sealed}")).cookie("theme"), None, "a rotated key");
    }

    #[test]
    fn a_value_too_large_is_a_500() {
        native::reset();
        let res = Response::status(204).cookie(Cookie::new("big", "x".repeat(5000)));
        assert_eq!((res.status_code(), res.header("set-cookie")), (500, None));
    }

    #[test]
    fn files_and_pages() {
        let dir = std::env::temp_dir().join(format!("islas-pages-{}", islas_sdk::random::token()));
        std::fs::create_dir_all(dir.join("admin")).unwrap();
        std::fs::write(dir.join("admin/index.html"), "<h1>admin</h1>").unwrap();
        std::fs::write(dir.join("admin/page"), "<p>no extension</p>").unwrap();
        crate::testing::with_static(&dir);
        let page = Response::page("admin/index.html").unwrap();
        assert_eq!((page.status_code(), page.body()), (200, &b""[..]), "the host sends the file");
        assert_eq!(page.header("x-islas-file"), Some("/static/admin/index.html"));
        assert_eq!(page.header("content-type"), Some("text/html; charset=utf-8"));
        let file = Response::file("admin/page").unwrap();
        assert_eq!((file.header("x-islas-file"), file.header("content-type")), (Some("/static/admin/page"), None));
        for bad in ["admin", "../secrets.json", "/etc/passwd", "admin/missing.html"] {
            assert!(matches!(Response::file(bad), Err(Error::NotFound)), "{bad}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn data_files_and_downloads() {
        let path = crate::data::path("exports/notes.csv");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "id\n1\n").unwrap();
        let res = Response::download("exports/notes.csv", "notes.csv").unwrap();
        assert_eq!(res.header("x-islas-file"), Some("/data/exports/notes.csv"));
        assert_eq!(res.header("content-disposition"), Some("attachment; filename=\"notes.csv\""));
        assert!(matches!(Response::data_file("exports/missing.csv"), Err(Error::NotFound)));
        assert!(matches!(Response::data_file("../static/x"), Err(Error::NotFound)));
        let text = Response::text("a,b").attachment("x.csv");
        assert_eq!((text.body(), text.header("content-disposition")), (&b"a,b"[..], Some("attachment; filename=\"x.csv\"")));
    }
}
