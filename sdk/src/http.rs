//! HTTP: turning the listener on, and requests and responses with whole
//! bodies in memory (no streaming in E1; the host caps bodies at 16 MiB).

use crate::bindings::wasi::http::types::{
    Fields, IncomingBody, IncomingRequest, Method as WasiMethod, OutgoingBody, OutgoingResponse,
    ResponseOutparam, Scheme,
};
use crate::bindings::wasi::io::streams::StreamError;

pub use crate::bindings::platform::app::http::Error;

/// Asks the host to open the app's HTTP listener once `start` returns. Call
/// it from `start`; anywhere else it answers [`Error::NotStarting`]. Where it
/// listens is the platform's business: `localhost:3000` under `islas run`
/// (another port with `--port`), the app's own socket behind
/// `<name>.<domain>` on the server.
pub fn listen() -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") {
        crate::bindings::platform::app::http::listen()
    } else {
        Ok(())
    }
}

/// An HTTP method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Delete,
    Connect,
    Options,
    Trace,
    Patch,
    Other(String),
}

impl From<WasiMethod> for Method {
    fn from(method: WasiMethod) -> Self {
        match method {
            WasiMethod::Get => Method::Get,
            WasiMethod::Head => Method::Head,
            WasiMethod::Post => Method::Post,
            WasiMethod::Put => Method::Put,
            WasiMethod::Delete => Method::Delete,
            WasiMethod::Connect => Method::Connect,
            WasiMethod::Options => Method::Options,
            WasiMethod::Trace => Method::Trace,
            WasiMethod::Patch => Method::Patch,
            WasiMethod::Other(other) => Method::Other(other),
        }
    }
}

/// An incoming request with its whole body read.
#[derive(Debug, Clone)]
pub struct Request {
    method: Method,
    scheme: Option<String>,
    authority: Option<String>,
    path: String,
    query: Option<String>,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

impl Request {
    /// A request with no headers and no body, for tests.
    pub fn new(method: Method, path_with_query: &str) -> Self {
        let (path, query) = match path_with_query.split_once('?') {
            Some((path, query)) => (path.to_string(), Some(query.to_string())),
            None => (path_with_query.to_string(), None),
        };
        Request { method, scheme: None, authority: None, path, query, headers: Vec::new(), body: Vec::new() }
    }

    /// Adds a header, for tests.
    pub fn with_header(mut self, name: &str, value: impl Into<Vec<u8>>) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }

    /// Sets the body, for tests.
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn method(&self) -> &Method {
        &self.method
    }

    /// `http` or `https`, as the client used it: behind Caddy, what Caddy
    /// says; locally, `http`.
    pub fn scheme(&self) -> Option<&str> {
        self.scheme.as_deref()
    }

    /// The host (and port) the client asked for: behind Caddy, what Caddy
    /// says; locally, the `Host` header.
    pub fn authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }

    /// The path, without the query.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The query string after `?`, undecoded.
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// The first value of a header, by case-insensitive name, if it is UTF-8.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, value)| std::str::from_utf8(value).ok())
    }

    /// Every header, names in lower case, in the order received.
    pub fn headers(&self) -> &[(String, Vec<u8>)] {
        &self.headers
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn into_body(self) -> Vec<u8> {
        self.body
    }

    /// Reads a request from the host: head, then the whole body.
    fn read(request: IncomingRequest) -> Result<Self, String> {
        let method = request.method().into();
        let path_with_query = request.path_with_query().unwrap_or_else(|| "/".to_string());
        let mut req = Request::new(method, &path_with_query);
        req.scheme = request.scheme().map(|scheme| match scheme {
            Scheme::Http => "http".to_string(),
            Scheme::Https => "https".to_string(),
            Scheme::Other(other) => other,
        });
        req.authority = request.authority();
        req.headers = request.headers().entries();

        let body = request.consume().map_err(|()| "request body already taken")?;
        let stream = body.stream().map_err(|()| "request body stream already taken")?;
        loop {
            match stream.blocking_read(64 * 1024) {
                Ok(chunk) => req.body.extend_from_slice(&chunk),
                Err(StreamError::Closed) => break,
                Err(StreamError::LastOperationFailed(e)) => {
                    return Err(format!("reading the request body: {}", e.to_debug_string()));
                }
            }
        }
        drop(stream);
        IncomingBody::finish(body);
        Ok(req)
    }
}

/// An outgoing response with its whole body.
#[derive(Debug, Clone)]
pub struct Response {
    status: u16,
    headers: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

impl Response {
    /// An empty response with this status.
    pub fn new(status: u16) -> Self {
        Response { status, headers: Vec::new(), body: Vec::new() }
    }

    /// 200 with a `text/plain` body.
    pub fn text(body: impl Into<String>) -> Self {
        Response::new(200)
            .with_header("content-type", "text/plain; charset=utf-8")
            .with_body(body.into())
    }

    /// 200 with an `application/octet-stream` body.
    pub fn bytes(body: impl Into<Vec<u8>>) -> Self {
        Response::new(200).with_header("content-type", "application/octet-stream").with_body(body)
    }

    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    /// Adds a header (a second one with the same name adds a value).
    pub fn with_header(mut self, name: &str, value: impl Into<Vec<u8>>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

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

    /// Sends the response to the host. A header wasi:http refuses, or a bad
    /// status, turns into a plain 500 and a line on stderr.
    fn send(self, out: ResponseOutparam) {
        let (response, body) = match self.outgoing() {
            Ok(parts) => parts,
            Err(e) => {
                eprintln!("islas-sdk: bad response: {e}");
                return Response::new(500).send(out);
            }
        };
        ResponseOutparam::set(out, Ok(response));
        finish(body, &self.body);
    }

    fn outgoing(&self) -> Result<(OutgoingResponse, OutgoingBody), String> {
        let headers = Fields::from_list(&self.headers).map_err(|e| format!("headers: {e:?}"))?;
        let response = OutgoingResponse::new(headers);
        response
            .set_status_code(self.status)
            .map_err(|()| format!("status {}", self.status))?;
        let body = response.body().map_err(|()| "response body already taken")?;
        Ok((response, body))
    }
}

/// Writes the whole body and finishes it. `blocking-write-and-flush` takes
/// at most 4096 bytes per call.
fn finish(body: OutgoingBody, bytes: &[u8]) {
    if let Ok(stream) = body.write() {
        for chunk in bytes.chunks(4096) {
            if let Err(e) = stream.blocking_write_and_flush(chunk) {
                eprintln!("islas-sdk: writing the response body: {e:?}");
                break;
            }
        }
        drop(stream);
    }
    if let Err(e) = OutgoingBody::finish(body, None) {
        eprintln!("islas-sdk: finishing the response body: {e:?}");
    }
}

/// Runs one HTTP event: reads the request, calls the handler, sends the
/// response. Called by the code `app!` generates.
#[doc(hidden)]
pub fn serve(request: IncomingRequest, out: ResponseOutparam, handler: impl FnOnce(Request) -> Response) {
    let response = match Request::read(request) {
        Ok(request) => handler(request),
        Err(e) => {
            eprintln!("islas-sdk: {e}");
            Response::text("bad request").with_status(400)
        }
    };
    response.send(out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_splits_path_and_query() {
        let req = Request::new(Method::Get, "/notes?limit=5&after=3")
            .with_header("Content-Type", "text/plain");
        assert_eq!(req.path(), "/notes");
        assert_eq!(req.query(), Some("limit=5&after=3"));
        assert_eq!(req.header("content-type"), Some("text/plain"));
        assert_eq!(Request::new(Method::Get, "/").query(), None);
    }

    #[test]
    fn response_builders() {
        let res = Response::text("hello").with_status(201);
        assert_eq!(res.status(), 201);
        assert_eq!(res.body(), b"hello");
        assert_eq!(res.header("Content-Type"), Some("text/plain; charset=utf-8"));
    }
}
