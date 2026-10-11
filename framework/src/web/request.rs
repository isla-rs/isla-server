//! The request a controller gets: the SDK's, plus the route's parameters.

use std::net::IpAddr;
use std::str::FromStr;

use serde::de::DeserializeOwned;

use super::{Error, Method, cookie};

/// An incoming request with its whole body, and the path parameters of the
/// route it matched.
#[derive(Debug, Clone)]
pub struct Request {
    raw: islas_sdk::Request,
    params: Vec<(String, String)>,
}

impl Request {
    pub(crate) fn new(raw: islas_sdk::Request, params: Vec<(String, String)>) -> Self {
        Request { raw, params }
    }

    pub fn method(&self) -> &Method {
        self.raw.method()
    }

    /// The path as received (percent-encoded), without the query.
    pub fn path(&self) -> &str {
        self.raw.path()
    }

    /// Path parameter `name` (`:name` or `*name` in the route), decoded and
    /// parsed: `req.param::<i64>("id")?`. One that doesn't parse is a 400.
    pub fn param<T: FromStr>(&self, name: &str) -> Result<T, Error> {
        let Some((_, value)) = self.params.iter().find(|(n, _)| n == name) else {
            return Err(Error::internal(format_args!("the route has no parameter `{name}`")));
        };
        value.parse().map_err(|_| Error::BadRequest(format!("bad {name}: {value:?}")))
    }

    /// The query string read into `T` (`?limit=5&tag=a`); a missing or
    /// malformed field is a 400. `Option` fields may be absent.
    pub fn query<T: DeserializeOwned>(&self) -> Result<T, Error> {
        serde_urlencoded::from_str(self.raw.query().unwrap_or(""))
            .map_err(|e| Error::BadRequest(format!("bad query: {e}")))
    }

    /// The first value of a header, by case-insensitive name, if it is UTF-8.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.raw.header(name)
    }

    /// Every header, names in lower case, in the order received.
    pub fn headers(&self) -> &[(String, Vec<u8>)] {
        self.raw.headers()
    }

    pub fn body(&self) -> &[u8] {
        self.raw.body()
    }

    /// The body read from JSON into `T`: 400 when it isn't that JSON, 415
    /// when the request says it is something else (no `Content-Type` is
    /// taken as JSON).
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        self.expect_type(|t| t == "application/json" || t.ends_with("+json"), "application/json")?;
        serde_json::from_slice(self.body()).map_err(|e| Error::BadRequest(format!("bad JSON: {e}")))
    }

    /// The body read from a form (`application/x-www-form-urlencoded`) into
    /// `T`: 400 when a field is missing or malformed, 415 when the request
    /// says it is something else.
    pub fn form<T: DeserializeOwned>(&self) -> Result<T, Error> {
        self.expect_type(|t| t == "application/x-www-form-urlencoded", "application/x-www-form-urlencoded")?;
        serde_urlencoded::from_bytes(self.body()).map_err(|e| Error::BadRequest(format!("bad form: {e}")))
    }

    /// Cookie `name`, set with [`Response::cookie`](super::Response::cookie)
    /// and opened here: `None` when there is no such cookie or its value
    /// isn't one this app sealed under this name (changed, moved from
    /// another cookie, made under a rotated key).
    pub fn cookie(&self, name: &str) -> Option<String> {
        let sealed = self.cookie_plain(name)?;
        String::from_utf8(islas_sdk::crypto::open(name, &sealed)?).ok()
    }

    /// Cookie `name` as the browser sent it (percent-decoded), unsealed: for
    /// a cookie the frontend sets or must read.
    pub fn cookie_plain(&self, name: &str) -> Option<String> {
        let headers = self.headers().iter().filter(|(n, _)| n == "cookie");
        cookie::find(headers.filter_map(|(_, value)| std::str::from_utf8(value).ok()), name)
    }

    /// The client's address: the last `X-Forwarded-For` entry, which the
    /// host sets (from Caddy on the server, from the peer locally).
    pub fn client_ip(&self) -> Option<IpAddr> {
        self.header("x-forwarded-for")?.rsplit(',').next()?.trim().parse().ok()
    }

    /// The id the host gave this request, also in its log lines and on the
    /// response (`X-Request-Id`).
    pub fn request_id(&self) -> Option<&str> {
        self.header("x-request-id")
    }

    /// `http` or `https`, as the client used it.
    pub fn scheme(&self) -> Option<&str> {
        self.raw.scheme()
    }

    /// The host (and port) the client asked for.
    pub fn authority(&self) -> Option<&str> {
        self.raw.authority()
    }

    /// The SDK's request underneath.
    pub fn raw(&self) -> &islas_sdk::Request {
        &self.raw
    }

    /// 415 unless the media type passes `ok` or there is none.
    fn expect_type(&self, ok: impl Fn(&str) -> bool, expected: &str) -> Result<(), Error> {
        let Some(content_type) = self.header("content-type") else { return Ok(()) };
        let media = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        if ok(&media) {
            Ok(())
        } else {
            Err(Error::UnsupportedMediaType(format!("expected {expected}, got {media}")))
        }
    }
}

/// `%XX` decoded; `None` for a bad escape or bytes that aren't UTF-8.
pub(crate) fn percent_decode(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_string());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    fn req(raw: islas_sdk::Request) -> Request {
        Request::new(raw, vec![("id".into(), "42".into()), ("name".into(), "a b".into())])
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct Note {
        text: String,
    }

    #[test]
    fn params_parse_or_400() {
        let r = req(islas_sdk::Request::new(Method::Get, "/x"));
        assert_eq!(r.param::<i64>("id").unwrap(), 42);
        assert_eq!(r.param::<String>("name").unwrap(), "a b");
        assert_eq!(r.param::<i64>("name").unwrap_err().status(), 400);
        assert_eq!(r.param::<i64>("nope").unwrap_err().status(), 500, "a route without it is the app's bug");
    }

    #[test]
    fn query_into_a_type() {
        #[derive(Deserialize, Debug)]
        struct Page {
            limit: u32,
            tag: Option<String>,
        }
        let r = req(islas_sdk::Request::new(Method::Get, "/x?limit=5&tag=a%20b"));
        let page: Page = r.query().unwrap();
        assert_eq!((page.limit, page.tag.as_deref()), (5, Some("a b")));
        let r = req(islas_sdk::Request::new(Method::Get, "/x?limit=many"));
        assert_eq!(r.query::<Page>().unwrap_err().status(), 400);
    }

    #[test]
    fn json_body_and_its_type() {
        let raw = islas_sdk::Request::new(Method::Post, "/x").with_body(r#"{"text":"hi"}"#);
        assert_eq!(req(raw.clone()).json::<Note>().unwrap(), Note { text: "hi".into() }, "no type is JSON");
        let typed = raw.clone().with_header("content-type", "application/json; charset=utf-8");
        assert!(req(typed).json::<Note>().is_ok());
        let text = raw.clone().with_header("content-type", "text/plain");
        assert_eq!(req(text).json::<Note>().unwrap_err().status(), 415);
        let bad = islas_sdk::Request::new(Method::Post, "/x").with_body("{");
        assert_eq!(req(bad).json::<Note>().unwrap_err().status(), 400);
    }

    #[test]
    fn form_body() {
        let raw = islas_sdk::Request::new(Method::Post, "/x")
            .with_header("content-type", "application/x-www-form-urlencoded")
            .with_body("text=a+b%21");
        assert_eq!(req(raw).form::<Note>().unwrap(), Note { text: "a b!".into() });
        let json = islas_sdk::Request::new(Method::Post, "/x").with_header("content-type", "application/json");
        assert_eq!(req(json).form::<Note>().unwrap_err().status(), 415);
    }

    #[test]
    fn cookies_ip_and_id() {
        let raw = islas_sdk::Request::new(Method::Get, "/x")
            .with_header("cookie", "theme=dark; tz=UTC")
            .with_header("x-forwarded-for", "203.0.113.9, 10.0.0.1")
            .with_header("x-request-id", "0123456789ab");
        let r = req(raw);
        assert_eq!(r.cookie_plain("tz").as_deref(), Some("UTC"));
        assert_eq!(r.client_ip(), Some("10.0.0.1".parse().unwrap()));
        assert_eq!(r.request_id(), Some("0123456789ab"));
    }

    #[test]
    fn decoding() {
        assert_eq!(percent_decode("a%20b%2Fc").as_deref(), Some("a b/c"));
        assert_eq!(percent_decode("100%"), None);
        assert_eq!(percent_decode("%ff"), None, "not UTF-8");
    }
}
