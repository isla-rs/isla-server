//! Why a request failed, as a status and `{"error": "…"}`.

use std::fmt;

use islas_sdk::{crypto, db, fetch, kv};

use super::{Method, Response, method_name};

/// A failed request. `?` turns the platform's errors into one: a busy
/// database is a 503, an outbound request that failed a 502, anything else
/// an [`Error::Internal`]. Internal errors (and bad gateways) are logged
/// with the method and path; the client gets only the status and a word.
#[derive(Debug)]
pub enum Error {
    /// 400, with the message.
    BadRequest(String),
    /// 401.
    Unauthorized,
    /// 403.
    Forbidden,
    /// 404.
    NotFound,
    /// 409, with the message.
    Conflict(String),
    /// 415, with the message (a body that isn't JSON or a form, as asked).
    UnsupportedMediaType(String),
    /// Any other status the client should see, with its message.
    Status(u16, String),
    /// 502: a service the app called failed. Logged, not sent.
    BadGateway(anyhow::Error),
    /// 500: the app's own failure. Logged, not sent.
    Internal(anyhow::Error),
}

impl Error {
    /// An [`Error::Internal`] with this message.
    pub fn internal(message: impl fmt::Display) -> Self {
        Error::Internal(anyhow::anyhow!("{message}"))
    }

    pub fn status(&self) -> u16 {
        match self {
            Error::BadRequest(_) => 400,
            Error::Unauthorized => 401,
            Error::Forbidden => 403,
            Error::NotFound => 404,
            Error::Conflict(_) => 409,
            Error::UnsupportedMediaType(_) => 415,
            Error::Status(status, _) => *status,
            Error::BadGateway(_) => 502,
            Error::Internal(_) => 500,
        }
    }

    /// What the client is told.
    fn message(&self) -> &str {
        match self {
            Error::BadRequest(message)
            | Error::Conflict(message)
            | Error::UnsupportedMediaType(message)
            | Error::Status(_, message) => message,
            Error::Unauthorized => "unauthorized",
            Error::Forbidden => "forbidden",
            Error::NotFound => "not found",
            Error::BadGateway(_) => "bad gateway",
            Error::Internal(_) => "internal error",
        }
    }

    /// The response for this error; an internal one is logged first (on
    /// stderr, which the host writes to the app's log as an error).
    pub(crate) fn into_response(self, method: &Method, path: &str) -> Response {
        if let Error::Internal(e) | Error::BadGateway(e) = &self {
            eprintln!("{} {path}: {e:#}", method_name(method));
        }
        Response::json(&serde_json::json!({ "error": self.message() })).with_status(self.status())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Internal(e) | Error::BadGateway(e) => write!(f, "{} {}: {e:#}", self.status(), self.message()),
            _ => write!(f, "{} {}", self.status(), self.message()),
        }
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Internal(e)
    }
}

impl From<db::Error> for Error {
    fn from(e: db::Error) -> Self {
        match e {
            db::Error::Busy => Error::Status(503, "the database is busy, try again".to_string()),
            db::Error::Sql(message) => Error::internal(format_args!("database: {message}")),
            db::Error::TooLarge => Error::internal("database: the result is over the size limit"),
            db::Error::Transaction(message) => Error::internal(format_args!("database: transaction: {message}")),
            db::Error::Unavailable(message) => Error::internal(format_args!("database: unavailable: {message}")),
        }
    }
}

impl From<kv::Error> for Error {
    fn from(e: kv::Error) -> Self {
        Error::Internal(e.into())
    }
}

impl From<crypto::Error> for Error {
    fn from(e: crypto::Error) -> Self {
        Error::Internal(e.into())
    }
}

impl From<fetch::Error> for Error {
    fn from(e: fetch::Error) -> Self {
        match e {
            // The app's own mistakes: its URL, its allowlist, its JSON.
            fetch::Error::BadUrl(_)
            | fetch::Error::Denied
            | fetch::Error::TooMany
            | fetch::Error::Json(_)
            | fetch::Error::Unavailable => Error::Internal(anyhow::anyhow!("fetch: {e}")),
            _ => Error::BadGateway(anyhow::anyhow!("fetch: {e}")),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Internal(anyhow::anyhow!("JSON: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(e: Error) -> (u16, String) {
        let res = e.into_response(&Method::Get, "/x");
        (res.status_code(), String::from_utf8(res.body().to_vec()).unwrap())
    }

    #[test]
    fn statuses_and_bodies() {
        assert_eq!(body(Error::BadRequest("text is empty".into())), (400, r#"{"error":"text is empty"}"#.into()));
        assert_eq!(body(Error::NotFound), (404, r#"{"error":"not found"}"#.into()));
        assert_eq!(body(Error::Status(503, "later".into())), (503, r#"{"error":"later"}"#.into()));
    }

    #[test]
    fn internals_are_not_sent() {
        let e: Error = db::Error::Sql("no such table: secret_plans".into()).into();
        assert_eq!(body(e), (500, r#"{"error":"internal error"}"#.into()));
        let e: Error = fetch::Error::Dns.into();
        assert_eq!(body(e), (502, r#"{"error":"bad gateway"}"#.into()));
        let e: Error = db::Error::Busy.into();
        assert_eq!(e.status(), 503);
    }
}
