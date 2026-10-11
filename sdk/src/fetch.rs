//! Requests to other services. An app has no sockets: the host makes each
//! request for it (`wasi:http/outgoing-handler`), within the app's
//! `[outbound] allow`.
//!
//! ```ignore
//! use islas_sdk::fetch;
//!
//! let user: User = fetch::get("https://api.github.com/users/octocat")
//!     .header("accept", "application/vnd.github+json")
//!     .send()?
//!     .json()?;
//!
//! // Several at once: start them all, then wait for each. The handler keeps
//! // running meanwhile; the requests go on while it calls db or kv too.
//! let a = fetch::get(url_a).start()?;
//! let b = fetch::post(url_b).json(&body).start()?;
//! let rows = db::query("select ...", &[])?;
//! let (a, b) = (a.wait()?, b.wait()?);
//! ```
//!
//! [`Request::start`] sends a request and returns a [`Pending`] at once;
//! [`Pending::wait`] waits for its whole response (this event only: other
//! requests to the app go on). The host's answer, a refusal included
//! ([`Error::Denied`], [`Error::TooMany`]), comes with `wait`; `start`
//! fails only for a bad URL or body. [`Request::send`] is both. A `Pending`
//! dropped before `wait`, or still pending when the handler returns, is
//! cancelled and its connection closed. Every wait counts against the
//! event's deadline.
//!
//! - **Responses** come whole, the body at most 16 MiB
//!   ([`Error::TooLarge`]). A 4xx or 5xx is a normal [`Response`], not an
//!   error.
//! - **Redirects** are followed for `GET` and `HEAD`, at most 10; a hop to
//!   another origin drops the credential headers (`Authorization`,
//!   `Cookie`, ...). Other methods get the 3xx as their response.
//! - **Headers.** The host owns `Host`, `Connection`, `Transfer-Encoding`
//!   and the other hop-by-hop headers, and this module `Content-Length`: an
//!   app's are dropped. The host adds `User-Agent: islas/<version> (<app>)`
//!   unless the app sets one.
//! - **Limits.** At most 64 requests open per event ([`Error::TooMany`]).
//!   Connect 30 s, the response head 30 s after it, 60 s without a body
//!   byte; [`Request::timeout`] sets shorter ones.
//!
//! Built natively (for `cargo test`), [`Request::start`] answers
//! [`Error::Unavailable`].

use std::fmt;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::bindings::wasi::http::outgoing_handler;
use crate::bindings::wasi::http::types::{
    ErrorCode, Fields, FutureIncomingResponse, IncomingBody, IncomingResponse, Method as WasiMethod, OutgoingBody,
    OutgoingRequest, RequestOptions, Scheme,
};
use crate::bindings::wasi::io::streams::{OutputStream, StreamError};
use crate::http::Method;

/// The largest response body read.
pub const MAX_BODY: usize = 16 << 20;
/// Redirect hops followed for one request.
pub const MAX_REDIRECTS: u8 = 10;
/// Headers the host or this module sets; an app's are dropped.
const OWNED_HEADERS: [&str; 9] = [
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "keep-alive",
    "upgrade",
    "proxy-connection",
    "proxy-authenticate",
    "http2-settings",
];
/// Headers dropped when a redirect leads to another origin.
const CREDENTIAL_HEADERS: [&str; 5] = ["authorization", "cookie", "cookie2", "proxy-authorization", "www-authenticate"];

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Not an absolute `http://` or `https://` URL with a host (or one with
    /// user info): the URL.
    BadUrl(String),
    /// The host is not in the app's `[outbound] allow`.
    Denied,
    /// 64 requests are open in this event already.
    TooMany,
    /// The host name could not be resolved.
    Dns,
    /// No connection: refused, unreachable, or closed early.
    Connect,
    /// The TLS handshake failed (a bad certificate, among others).
    Tls,
    /// No connection, response head or body bytes in time.
    Timeout,
    /// The response body is over [`MAX_BODY`].
    TooLarge,
    /// More than [`MAX_REDIRECTS`] redirects.
    TooManyRedirects,
    /// The request body could not be made, or the response body is not the
    /// JSON asked for: serde's message.
    Json(String),
    /// Anything else the host reported, in its words.
    Http(String),
    /// Not running on the islas host (a native build).
    Unavailable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadUrl(url) => write!(f, "not an http:// or https:// URL with a host: {url}"),
            Error::Denied => f.write_str("the host is not in the app's [outbound] allow"),
            Error::TooMany => f.write_str("too many requests open in this event (64)"),
            Error::Dns => f.write_str("the host name could not be resolved"),
            Error::Connect => f.write_str("could not connect"),
            Error::Tls => f.write_str("the TLS handshake failed"),
            Error::Timeout => f.write_str("timed out"),
            Error::TooLarge => write!(f, "the response body is over {} MiB", MAX_BODY >> 20),
            Error::TooManyRedirects => write!(f, "more than {MAX_REDIRECTS} redirects"),
            Error::Json(e) => write!(f, "JSON: {e}"),
            Error::Http(e) => write!(f, "HTTP: {e}"),
            Error::Unavailable => f.write_str(crate::NATIVE),
        }
    }
}

impl std::error::Error for Error {}

impl From<ErrorCode> for Error {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::HttpRequestDenied => Error::Denied,
            ErrorCode::ConnectionLimitReached => Error::TooMany,
            ErrorCode::DnsTimeout | ErrorCode::DnsError(_) => Error::Dns,
            ErrorCode::DestinationNotFound
            | ErrorCode::DestinationUnavailable
            | ErrorCode::DestinationIpProhibited
            | ErrorCode::DestinationIpUnroutable
            | ErrorCode::ConnectionRefused
            | ErrorCode::ConnectionTerminated => Error::Connect,
            ErrorCode::TlsProtocolError | ErrorCode::TlsCertificateError | ErrorCode::TlsAlertReceived(_) => Error::Tls,
            ErrorCode::ConnectionTimeout
            | ErrorCode::ConnectionReadTimeout
            | ErrorCode::ConnectionWriteTimeout
            | ErrorCode::HttpResponseTimeout => Error::Timeout,
            other => Error::Http(format!("{other:?}")),
        }
    }
}

/// A `GET` request to `url`.
pub fn get(url: &str) -> Request {
    request(Method::Get, url)
}

pub fn head(url: &str) -> Request {
    request(Method::Head, url)
}

pub fn post(url: &str) -> Request {
    request(Method::Post, url)
}

pub fn put(url: &str) -> Request {
    request(Method::Put, url)
}

pub fn patch(url: &str) -> Request {
    request(Method::Patch, url)
}

pub fn delete(url: &str) -> Request {
    request(Method::Delete, url)
}

/// A request with any method to `url`, an absolute `http://` or
/// `https://` URL. Nothing is sent until [`start`](Request::start) or
/// [`send`](Request::send).
pub fn request(method: Method, url: &str) -> Request {
    Request { method, url: url.to_string(), headers: Vec::new(), body: Vec::new(), timeout: None, error: None }
}

/// A request being built.
#[derive(Debug, Clone)]
#[must_use = "a request is sent by `start` or `send`"]
pub struct Request {
    method: Method,
    url: String,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
    timeout: Option<Duration>,
    /// A builder step that failed (`json`), reported by `start`.
    error: Option<Error>,
}

impl Request {
    /// Adds a header (a name can repeat). One the host owns, or
    /// `Content-Length`, is dropped.
    pub fn header(mut self, name: &str, value: impl Into<Vec<u8>>) -> Self {
        let name = name.to_ascii_lowercase();
        if !OWNED_HEADERS.contains(&name.as_str()) {
            self.headers.push((name, value.into()));
        }
        self
    }

    /// `Authorization: Bearer <token>`.
    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", format!("Bearer {token}"))
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// `value` as JSON, with `Content-Type: application/json` unless one is
    /// set.
    pub fn json<T: Serialize + ?Sized>(mut self, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => self.body = body,
            Err(e) => self.error = Some(Error::Json(e.to_string())),
        }
        if !self.headers.iter().any(|(name, _)| name == "content-type") {
            self.headers.push(("content-type".to_string(), b"application/json".to_vec()));
        }
        self
    }

    /// At most this long to connect, for the response head, and between
    /// body bytes, each; the host's limits are the most.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Sends the request and returns at once; the response comes with
    /// [`Pending::wait`]. A request body over 1 MiB is handed over in part
    /// here and the rest in `wait`.
    pub fn start(mut self) -> Result<Pending, Error> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        let url = Url::parse(&self.url)?;
        if !cfg!(target_arch = "wasm32") {
            return Err(Error::Unavailable);
        }
        let body = std::mem::take(&mut self.body);
        let (future, upload) = self.sent(&url, body)?;
        Ok(Pending { request: self, url, hops: 0, future, upload })
    }

    /// [`start`](Request::start), then [`wait`](Pending::wait).
    pub fn send(self) -> Result<Response, Error> {
        self.start()?.wait()
    }

    /// Hands the request to the host, with the body as far as the host
    /// takes it without waiting.
    fn sent(&self, url: &Url, body: Vec<u8>) -> Result<(FutureIncomingResponse, Option<Upload>), Error> {
        let mut headers = self.headers.clone();
        let sends_body = !body.is_empty() || matches!(self.method, Method::Post | Method::Put | Method::Patch);
        if sends_body {
            headers.push(("content-length".to_string(), body.len().to_string().into_bytes()));
        }
        let fields = Fields::from_list(&headers).map_err(|e| Error::Http(format!("headers: {e:?}")))?;
        let request = OutgoingRequest::new(fields);
        let scheme = if url.https { Scheme::Https } else { Scheme::Http };
        let set = request
            .set_method(&wasi_method(&self.method))
            .and_then(|()| request.set_scheme(Some(&scheme)))
            .and_then(|()| request.set_authority(Some(&url.authority)))
            .and_then(|()| request.set_path_with_query(Some(&url.path)));
        set.map_err(|()| Error::BadUrl(url.to_string()))?;
        let outgoing = request.body().map_err(|()| Error::Http("request body already taken".to_string()))?;
        let options = self.timeout.map(|timeout| {
            let options = RequestOptions::new();
            let ns = Some(u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX));
            // An option the host doesn't support stays at its default.
            let _ = options.set_connect_timeout(ns);
            let _ = options.set_first_byte_timeout(ns);
            let _ = options.set_between_bytes_timeout(ns);
            options
        });
        let future = outgoing_handler::handle(request, options)?;
        let mut upload = Upload::new(outgoing, body)?;
        Ok((future, if upload.push(false)? { None } else { Some(upload) }))
    }
}

/// A request sent, its response not yet read: see [`Request::start`].
/// Dropping it cancels the request.
#[must_use = "dropping a Pending cancels its request"]
pub struct Pending {
    /// For the next hop of a redirect (its body was sent).
    request: Request,
    url: Url,
    hops: u8,
    future: FutureIncomingResponse,
    /// The request body still to hand over.
    upload: Option<Upload>,
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pending").field("method", &self.request.method).field("url", &self.url.to_string()).finish()
    }
}

impl Pending {
    /// Whether the response head has come, so `wait` only reads its body
    /// (or follows a redirect).
    pub fn is_ready(&self) -> bool {
        self.future.subscribe().ready()
    }

    /// Waits for the response, follows redirects, reads the whole body.
    pub fn wait(mut self) -> Result<Response, Error> {
        loop {
            if let Some(mut upload) = self.upload.take() {
                upload.push(true)?;
            }
            self.future.subscribe().block();
            let response = match self.future.get() {
                Some(Ok(result)) => result?,
                // `get` after `block` always has the answer.
                Some(Err(())) | None => return Err(Error::Http("the response was already taken".to_string())),
            };
            let status = response.status();
            let headers = response.headers().entries();
            let location = header(&headers, "location").map(str::to_string);
            let next = location.and_then(|location| redirect(&self.request, &self.url, status, &location));
            let Some(next) = next else {
                let body = read_body(response)?;
                return Ok(Response { status, headers, body, url: self.url.to_string() });
            };
            drop(response);
            let (url, headers) = next?;
            self.hops += 1;
            if self.hops > MAX_REDIRECTS {
                return Err(Error::TooManyRedirects);
            }
            self.request.headers = headers;
            let (future, upload) = self.request.sent(&url, Vec::new())?;
            (self.url, self.future, self.upload) = (url, future, upload);
        }
    }
}

/// A redirect's target and the headers to send there.
type Hop = (Url, Vec<(String, Vec<u8>)>);

/// The next hop for a response with `status` and `location`, if one is
/// followed: `GET` and `HEAD` only, the request's headers without the
/// credentials when the origin changes.
fn redirect(request: &Request, from: &Url, status: u16, location: &str) -> Option<Result<Hop, Error>> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) || !matches!(request.method, Method::Get | Method::Head) {
        return None;
    }
    let to = match from.join(location) {
        Ok(to) => to,
        Err(e) => return Some(Err(e)),
    };
    let mut headers = request.headers.clone();
    if !to.same_origin(from) {
        headers.retain(|(name, _)| !CREDENTIAL_HEADERS.contains(&name.as_str()));
    }
    Some(Ok((to, headers)))
}

/// A request body being handed to the host: in chunks of what its stream
/// takes, then finished.
struct Upload {
    /// Dropped before `body` is finished (a child resource).
    stream: Option<OutputStream>,
    body: Option<OutgoingBody>,
    bytes: Vec<u8>,
    at: usize,
}

impl Upload {
    fn new(body: OutgoingBody, bytes: Vec<u8>) -> Result<Self, Error> {
        let stream = body.write().map_err(|()| Error::Http("request body stream already taken".to_string()))?;
        Ok(Upload { stream: Some(stream), body: Some(body), bytes, at: 0 })
    }

    /// Writes what the stream takes; with `wait`, waits for room until all
    /// is written. Finishes the body once it is; returns whether it did.
    fn push(&mut self, wait: bool) -> Result<bool, Error> {
        let Some(stream) = &self.stream else { return Ok(true) };
        while self.at < self.bytes.len() {
            let room = match stream.check_write() {
                Ok(room) => room as usize,
                // The host ended the request: its response says why.
                Err(StreamError::Closed) => break,
                Err(StreamError::LastOperationFailed(e)) => return Err(Error::Http(e.to_debug_string())),
            };
            if room == 0 {
                if !wait {
                    return Ok(false);
                }
                stream.subscribe().block();
                continue;
            }
            let end = self.bytes.len().min(self.at + room);
            match stream.write(&self.bytes[self.at..end]) {
                Ok(()) => self.at = end,
                Err(StreamError::Closed) => break,
                Err(StreamError::LastOperationFailed(e)) => return Err(Error::Http(e.to_debug_string())),
            }
        }
        self.stream = None;
        if let Some(body) = self.body.take() {
            OutgoingBody::finish(body, None)?;
        }
        Ok(true)
    }
}

/// Reads a response body whole, within [`MAX_BODY`].
fn read_body(response: IncomingResponse) -> Result<Vec<u8>, Error> {
    let body = response.consume().map_err(|()| Error::Http("response body already taken".to_string()))?;
    let stream = body.stream().map_err(|()| Error::Http("response body stream already taken".to_string()))?;
    let mut bytes = Vec::new();
    loop {
        match stream.blocking_read(64 * 1024) {
            Ok(chunk) if bytes.len() + chunk.len() > MAX_BODY => return Err(Error::TooLarge),
            Ok(chunk) => bytes.extend_from_slice(&chunk),
            Err(StreamError::Closed) => break,
            // The host's error code, if it gave one.
            Err(StreamError::LastOperationFailed(e)) => {
                return Err(crate::bindings::wasi::http::types::http_error_code(&e)
                    .map_or_else(|| Error::Http(e.to_debug_string()), Error::from));
            }
        }
    }
    drop(stream);
    IncomingBody::finish(body);
    Ok(bytes)
}

/// A response with its whole body.
#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
    url: String,
}

impl Response {
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The first value of a header (any case), if present and text.
    pub fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }

    /// Every header, in the order received; a name can repeat.
    pub fn headers(&self) -> &[(String, Vec<u8>)] {
        &self.headers
    }

    /// The URL that answered, after redirects.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.body
    }

    /// The body as text, invalid UTF-8 replaced.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_slice(&self.body).map_err(|e| Error::Json(e.to_string()))
    }
}

fn header<'a>(headers: &'a [(String, Vec<u8>)], name: &str) -> Option<&'a str> {
    let (_, value) = headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name))?;
    std::str::from_utf8(value).ok()
}

fn wasi_method(method: &Method) -> WasiMethod {
    match method {
        Method::Get => WasiMethod::Get,
        Method::Head => WasiMethod::Head,
        Method::Post => WasiMethod::Post,
        Method::Put => WasiMethod::Put,
        Method::Delete => WasiMethod::Delete,
        Method::Connect => WasiMethod::Connect,
        Method::Options => WasiMethod::Options,
        Method::Trace => WasiMethod::Trace,
        Method::Patch => WasiMethod::Patch,
        Method::Other(other) => WasiMethod::Other(other.clone()),
    }
}

/// An absolute `http://` or `https://` URL, in the parts `wasi:http` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    https: bool,
    /// `host` or `host:port`, as written.
    authority: String,
    /// From the first `/`, with the query; never the fragment.
    path: String,
}

impl Url {
    fn parse(url: &str) -> Result<Url, Error> {
        let bad = || Error::BadUrl(url.to_string());
        let (scheme, rest) = url.split_once("://").ok_or_else(bad)?;
        let https = match scheme.to_ascii_lowercase().as_str() {
            "https" => true,
            "http" => false,
            _ => return Err(bad()),
        };
        let rest = rest.split('#').next().unwrap_or_default();
        let end = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(end);
        if authority.is_empty() || authority.contains('@') || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(bad());
        }
        let path = match path {
            "" => "/".to_string(),
            p if p.starts_with('?') => format!("/{p}"),
            p => p.to_string(),
        };
        Ok(Url { https, authority: authority.to_string(), path })
    }

    /// `location` (absolute, `//host/...`, `/path` or relative) against this
    /// URL.
    fn join(&self, location: &str) -> Result<Url, Error> {
        let location = location.trim();
        if location.contains("://") {
            return Url::parse(location);
        }
        let scheme = if self.https { "https" } else { "http" };
        if let Some(rest) = location.strip_prefix("//") {
            return Url::parse(&format!("{scheme}://{rest}"));
        }
        let path = if location.starts_with('/') {
            location.to_string()
        } else {
            let base = self.path.split('?').next().unwrap_or("/");
            let dir = &base[..base.rfind('/').map_or(0, |i| i + 1)];
            format!("{dir}{location}")
        };
        Url::parse(&format!("{scheme}://{}{path}", self.authority))
    }

    /// Same scheme, host and port (names without case).
    fn same_origin(&self, other: &Url) -> bool {
        self.https == other.https && self.host_port() == other.host_port()
    }

    /// The host in lower case and the port, the scheme's default if none.
    fn host_port(&self) -> (String, u16) {
        let authority = self.authority.to_ascii_lowercase();
        // After the last `:`, unless that is inside an IPv6 `[...]`.
        let split = authority.rsplit_once(':').filter(|(_, port)| !port.contains(']'));
        match split.and_then(|(host, port)| Some((host, port.parse().ok()?))) {
            Some((host, port)) => (host.to_string(), port),
            None => (authority, if self.https { 443 } else { 80 }),
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.https { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority, self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_parse() {
        let url = Url::parse("https://api.github.com/repos/x?page=2#top").unwrap();
        assert_eq!(url, Url { https: true, authority: "api.github.com".into(), path: "/repos/x?page=2".into() });
        assert_eq!(Url::parse("HTTP://localhost:8080").unwrap().path, "/");
        assert_eq!(Url::parse("http://h?q=1").unwrap().path, "/?q=1");
        for bad in ["ftp://x/", "http://", "http:///x", "localhost/x", "http://user:pw@h/", "http://h/a b"] {
            assert_eq!(Url::parse(bad), Err(Error::BadUrl(bad.to_string())), "{bad}");
        }
    }

    #[test]
    fn locations_join() {
        let from = Url::parse("https://a.com/x/y?q=1").unwrap();
        let join = |l: &str| from.join(l).unwrap().to_string();
        assert_eq!(join("http://b.com/z"), "http://b.com/z");
        assert_eq!(join("//b.com/z"), "https://b.com/z");
        assert_eq!(join("/z?k"), "https://a.com/z?k");
        assert_eq!(join("z"), "https://a.com/x/z");
    }

    #[test]
    fn origins() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(url("https://a.com/x").same_origin(&url("https://A.com:443/y")));
        assert!(!url("https://a.com/").same_origin(&url("http://a.com/")));
        assert!(!url("https://a.com/").same_origin(&url("https://a.com:8443/")));
        assert!(!url("https://a.com/").same_origin(&url("https://b.a.com/")));
    }

    #[test]
    fn redirects_drop_credentials_across_origins() {
        let req = get("https://a.com/x").bearer("t").header("Cookie", "c").header("Accept", "text/plain");
        let from = Url::parse(&req.url).unwrap();
        let names = |headers: Vec<(String, Vec<u8>)>| headers.into_iter().map(|(n, _)| n).collect::<Vec<_>>();

        let (to, headers) = redirect(&req, &from, 302, "/y").unwrap().unwrap();
        assert_eq!(to.to_string(), "https://a.com/y");
        assert_eq!(names(headers), ["authorization", "cookie", "accept"]);

        let (to, headers) = redirect(&req, &from, 301, "https://b.com/y").unwrap().unwrap();
        assert_eq!(to.to_string(), "https://b.com/y");
        assert_eq!(names(headers), ["accept"]);

        assert!(redirect(&req, &from, 200, "/y").is_none());
        assert!(redirect(&post("https://a.com/x"), &from, 307, "/y").is_none());
    }

    #[test]
    fn owned_headers_are_dropped() {
        let req = get("http://h/").header("Host", "evil").header("Content-Length", "9").header("X-Ok", "1");
        assert_eq!(req.headers, [("x-ok".to_string(), b"1".to_vec())]);
    }

    #[test]
    fn natively_unavailable() {
        assert_eq!(get("http://localhost/").send().unwrap_err(), Error::Unavailable);
        assert_eq!(get("nope").send().unwrap_err(), Error::BadUrl("nope".into()));
    }
}
