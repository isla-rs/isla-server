//! CSRF tokens for forms: a signed token bound to the visitor's session,
//! put in the form as the field `_csrf` (or sent as the header
//! `X-CSRF-Token`) and checked before the action runs. A page from another
//! origin can make the browser send the cookie but can't read a token.
//!
//! ```ignore
//! // Rendering the form:
//! let form = format!(r#"<input type="hidden" name="_csrf" value="{}">"#, csrf::token(&req)?);
//!
//! // The controller that takes forms: `protect` checks every POST, PUT,
//! // PATCH and DELETE and lets GET through.
//! impl Controller for Settings {
//!     fn before(req: &Request) -> web::Result<()> {
//!         csrf::protect(req)
//!     }
//! }
//! ```
//!
//! A token is `crypto::sign` under the label `csrf` with the session id as
//! its value (empty without a session), good for a day; it is checked with
//! `crypto::verify` and compared with the session of the request it comes
//! with, so a token made for one visitor is nothing for another. An API
//! that is called with JSON from the app's own JavaScript doesn't need
//! this: a cross-site form can't send JSON, and the session cookie is
//! `SameSite=Lax`.

use std::time::Duration;

use islas_sdk::crypto;

use super::{Error, Method, Request, session};

/// The form field the token goes in.
pub const FIELD: &str = "_csrf";
/// The header it may come in instead.
pub const HEADER: &str = "x-csrf-token";
const LABEL: &str = "csrf";
const TTL: Duration = Duration::from_secs(24 * 3600);

/// A token for the request's session, to put in a form.
pub fn token(req: &Request) -> Result<String, Error> {
    Ok(crypto::sign(LABEL, session::id(req).unwrap_or_default().as_bytes(), TTL)?)
}

/// Checks the request's token (the header, else the form field): a 403
/// without one, with one that doesn't verify, or with one made for another
/// session.
pub fn check(req: &Request) -> Result<(), Error> {
    let refused = || Error::Status(403, "missing or invalid CSRF token".to_string());
    let sent = req.header(HEADER).map(str::to_string).or_else(|| form_field(req, FIELD)).ok_or_else(refused)?;
    let bound = crypto::verify(LABEL, &sent).ok_or_else(refused)?;
    if bound == session::id(req).unwrap_or_default().as_bytes() { Ok(()) } else { Err(refused()) }
}

/// [`check`] for the methods that change something (anything but `GET`,
/// `HEAD` and `OPTIONS`): a controller's whole `before`.
pub fn protect(req: &Request) -> Result<(), Error> {
    match req.method() {
        Method::Get | Method::Head | Method::Options => Ok(()),
        _ => check(req),
    }
}

/// Field `name` of a form body, if the request is one.
fn form_field(req: &Request, name: &str) -> Option<String> {
    let content_type = req.header("content-type")?;
    let media = content_type.split(';').next().unwrap_or("").trim();
    if !media.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        return None;
    }
    let fields: Vec<(String, String)> = serde_urlencoded::from_bytes(req.body()).ok()?;
    fields.into_iter().find(|(field, _)| field == name).map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::App;
    use crate::testing::{self, native};
    use crate::web::{Controller, Response, Routes};

    struct Settings;

    impl Controller for Settings {
        fn before(req: &Request) -> Result<(), Error> {
            protect(req)
        }
    }

    fn form(req: Request) -> Result<Response, Error> {
        Ok(Response::text(token(&req)?))
    }

    fn save(_req: Request) -> Result<Response, Error> {
        Ok(Response::status(204))
    }

    fn login(_req: Request) -> Result<Response, Error> {
        session::start(Response::status(201), &"ann", Duration::from_secs(60))
    }

    fn app() -> App {
        App::new().routes(|r: &mut Routes| {
            r.post("/login", login);
            r.controller::<Settings>(|c| {
                c.get("settings", form);
                c.post("settings", save);
                c.delete("settings", save);
            });
        })
    }

    #[test]
    fn forms_need_their_token() {
        native::reset();
        let app = app();
        let cookie = testing::run_request(&app, testing::post("/login")).cookie(session::COOKIE).unwrap();
        let token = testing::run_request(&app, testing::get("/settings").cookie(session::COOKIE, &cookie)).text().to_string();
        let post = |token: &str, cookie: &str| {
            testing::post("/settings").cookie(session::COOKIE, cookie).form(&[("name", "x"), (FIELD, token)])
        };
        assert_eq!(testing::run_request(&app, post(&token, &cookie)).status(), 204, "the form field");
        let header = testing::post("/settings").cookie(session::COOKIE, &cookie).header(HEADER, &token);
        assert_eq!(testing::run_request(&app, header).status(), 204, "the header");
        assert_eq!(testing::run_request(&app, testing::get("/settings")).status(), 200, "GET passes without one");

        let no_token = testing::post("/settings").cookie(session::COOKIE, &cookie).form(&[("name", "x")]);
        let refused = testing::run_request(&app, no_token);
        assert_eq!((refused.status(), refused.text()), (403, r#"{"error":"missing or invalid CSRF token"}"#));
        assert_eq!(testing::run_request(&app, post("forged", &cookie)).status(), 403);
        assert_eq!(testing::run_request(&app, testing::delete("/settings").cookie(session::COOKIE, &cookie)).status(), 403);

        let other = testing::run_request(&app, testing::post("/login")).cookie(session::COOKIE).unwrap();
        assert_eq!(testing::run_request(&app, post(&token, &other)).status(), 403, "another visitor's token");
        let anonymous = testing::run_request(&app, testing::get("/settings")).text().to_string();
        assert_eq!(testing::run_request(&app, post(&anonymous, &cookie)).status(), 403, "a token made without a session");
        assert_eq!(testing::run_request(&app, testing::post("/settings").form(&[(FIELD, anonymous.as_str())])).status(), 204, "but fine without one");

        native::advance(Duration::from_secs(25 * 3600));
        let cookie = testing::run_request(&app, testing::post("/login")).cookie(session::COOKIE).unwrap();
        let old = testing::run_request(&app, testing::get("/settings").cookie(session::COOKIE, &cookie)).text().to_string();
        native::advance(Duration::from_secs(25 * 3600));
        assert_eq!(testing::run_request(&app, post(&old, &cookie)).status(), 403, "a token expires after a day");

        native::crypto::rotate();
        assert_eq!(testing::run_request(&app, post(&token, &cookie)).status(), 403, "another app's key: nothing opens");
    }
}
