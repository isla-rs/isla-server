//! What a service answers when it can't: a business error, and the one
//! place it becomes a status.
//!
//! A service is a plain function in the app's `services/`: it takes its
//! input, the rows it needs and `now` or generated ids as arguments, and
//! returns a value or an [`Error`]. It never calls `db`, `kv`, `fetch`,
//! `secrets`, the clock or `random`; the controller around it loads, calls
//! it, saves and answers (a read–decide–write inside `db::transaction`).
//! So a service is tested with a plain `#[test]` and no backend.
//!
//! ```ignore
//! use islas::service::{self, Error};
//!
//! pub fn rename_note(note: Note, title: &str, taken: &[String]) -> service::Result<Rename> {
//!     let title = title.trim();
//!     if title.is_empty() {
//!         return Err(Error::invalid("title is empty"));
//!     }
//!     if note.title.as_deref() == Some(title) {
//!         return Ok(Rename::Unchanged(note));
//!     }
//!     if taken.iter().any(|t| t == title) {
//!         return Err(Error::conflict(format!("a note titled {title:?} exists")));
//!     }
//!     Ok(Rename::Changed(Note { title: Some(title.to_string()), ..note }))
//! }
//! ```
//!
//! "Not found" is a model's answer, not a service's: `query_optional` →
//! `None` → `web::Error::NotFound` in the controller.

use std::fmt;

use crate::web;

/// Why a service refused. `?` in a controller turns it into the
/// [`web::Error`] with the matching status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The input can't be accepted as it is: 400, with the message.
    Invalid(String),
    /// The decision clashes with what already exists: 409, with the message.
    Conflict(String),
    /// The caller may not do this: 403.
    Forbidden,
}

impl Error {
    pub fn invalid(message: impl Into<String>) -> Self {
        Error::Invalid(message.into())
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Error::Conflict(message.into())
    }
}

/// What a service returns: `Result<T, Error>` by default.
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Invalid(message) | Error::Conflict(message) => f.write_str(message),
            Error::Forbidden => f.write_str("forbidden"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for web::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Invalid(message) => web::Error::BadRequest(message),
            Error::Conflict(message) => web::Error::Conflict(message),
            Error::Forbidden => web::Error::Forbidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;
    use crate::testing::{get, run_request};
    use crate::web::{Request, Response};

    fn decide(req: Request) -> web::Result {
        let answer: Result<&str> = match req.param::<String>("what")?.as_str() {
            "invalid" => Err(Error::invalid("text is empty")),
            "conflict" => Err(Error::conflict("a note titled \"x\" exists")),
            "forbidden" => Err(Error::Forbidden),
            _ => Ok("fine"),
        };
        Ok(Response::text(answer?))
    }

    #[test]
    fn a_business_error_becomes_a_status() {
        let app = App::new().routes(|r| {
            r.get("/:what", decide);
        });
        let answer = |what: &str| {
            let reply = run_request(&app, get(&format!("/{what}")));
            (reply.status(), reply.text().to_string())
        };
        assert_eq!(answer("invalid"), (400, r#"{"error":"text is empty"}"#.to_string()));
        assert_eq!(answer("conflict"), (409, r#"{"error":"a note titled \"x\" exists"}"#.to_string()));
        assert_eq!(answer("forbidden"), (403, r#"{"error":"forbidden"}"#.to_string()));
        assert_eq!(answer("ok"), (200, "fine".to_string()));
    }

    #[test]
    fn displays_its_message() {
        assert_eq!(Error::invalid("too long").to_string(), "too long");
        assert_eq!(Error::Forbidden.to_string(), "forbidden");
    }
}
