//! Sessions: a sealed cookie holding a random id, the data as JSON in the
//! app's KV under `session/<id>` with a TTL. Revocable (delete the entry),
//! nothing readable in the browser, nothing an app can get wrong.
//!
//! ```ignore
//! // Logging in: whatever the app wants to remember about the visitor.
//! #[derive(Serialize, Deserialize)]
//! struct User { name: String }
//!
//! fn create(req: Request) -> web::Result {
//!     let login: Login = req.json()?;
//!     session::start(Response::status(201), &User { name: login.user }, Duration::from_secs(3600))
//! }
//!
//! // A controller only a logged-in visitor may use.
//! impl Controller for Admin {
//!     fn before(req: &Request) -> web::Result<()> {
//!         session::require::<User>(req).map(|_| ())
//!     }
//! }
//!
//! // Logging out.
//! fn destroy(req: Request) -> web::Result {
//!     session::end(&req, Response::status(204))
//! }
//! ```
//!
//! A session can't be read in `start` or during a deploy's health check
//! (the KV store changes hands at the drain), which neither needs.

use std::time::Duration;

use islas_sdk::{kv, random};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Cookie, Error, Request, Response};

/// The cookie's name.
pub const COOKIE: &str = "session";
/// The KV keys sessions live under.
const PREFIX: &str = "session/";

/// Starts a session holding `data` for `ttl`: stores it and sets the
/// cookie on `res`. A visitor who already had one gets a new one; the old
/// entry expires on its own.
pub fn start<T: Serialize + ?Sized>(res: Response, data: &T, ttl: Duration) -> Result<Response, Error> {
    let id = random::token();
    kv::set_json_ttl(&key(&id), data, ttl)?;
    Ok(res.cookie(Cookie::new(COOKIE, id).max_age(ttl)))
}

/// The session's id, from the cookie; `None` without a valid one. The data
/// may still have expired: use [`get`] for it.
pub fn id(req: &Request) -> Option<String> {
    req.cookie(COOKIE).filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// The session's data, or `None` when the request has no session cookie,
/// the session expired or was ended, or its data isn't a `T` any more (an
/// older version's shape).
pub fn get<T: DeserializeOwned>(req: &Request) -> Result<Option<T>, Error> {
    let Some(id) = id(req) else { return Ok(None) };
    match kv::get_json(&key(&id)) {
        Ok(data) => Ok(data),
        Err(kv::Error::Json(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// [`get`], or a 401: what a controller's `before` calls.
pub fn require<T: DeserializeOwned>(req: &Request) -> Result<T, Error> {
    get(req)?.ok_or(Error::Unauthorized)
}

/// Replaces the session's data, keeping its id and cookie, for `ttl` more.
/// A 401 without a session.
pub fn update<T: Serialize + ?Sized>(req: &Request, data: &T, ttl: Duration) -> Result<(), Error> {
    let id = id(req).ok_or(Error::Unauthorized)?;
    kv::set_json_ttl(&key(&id), data, ttl)?;
    Ok(())
}

/// Ends the session: its data is deleted (any copy of the cookie is dead
/// at once) and the browser is told to drop the cookie.
pub fn end(req: &Request, res: Response) -> Result<Response, Error> {
    if let Some(id) = id(req) {
        kv::delete(&key(&id))?;
    }
    Ok(res.remove_cookie(COOKIE))
}

fn key(id: &str) -> String {
    format!("{PREFIX}{id}")
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::App;
    use crate::testing::{self, native};
    use crate::web::{self, Controller, Routes};

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct User {
        name: String,
    }

    struct Account;

    impl Controller for Account {
        fn before(req: &Request) -> web::Result<()> {
            require::<User>(req).map(|_| ())
        }
    }

    fn login(_req: Request) -> web::Result {
        start(Response::status(201), &User { name: "ann".into() }, Duration::from_secs(60))
    }

    fn me(req: Request) -> web::Result {
        Ok(Response::json(&get::<User>(&req)?))
    }

    fn rename(req: Request) -> web::Result {
        update(&req, &User { name: "bob".into() }, Duration::from_secs(60))?;
        Ok(Response::status(204))
    }

    fn logout(req: Request) -> web::Result {
        end(&req, Response::status(204))
    }

    fn app() -> App {
        App::new().routes(|r: &mut Routes| {
            r.post("/login", login).get("/me", me);
            r.controller::<Account>(|c| {
                c.get("account", me);
                c.post("account/rename", rename);
                c.delete("account", logout);
            });
        })
    }

    #[test]
    fn created_read_updated_revoked_and_expired() {
        native::reset();
        let app = app();
        assert_eq!(testing::run_request(&app, testing::get("/me")).text(), "null");
        assert_eq!(testing::run_request(&app, testing::get("/account")).status(), 401);

        let created = testing::run_request(&app, testing::post("/login"));
        assert_eq!(created.status(), 201);
        let cookie = created.cookie(COOKIE).unwrap();
        assert!(created.set_cookie(COOKIE).unwrap().ends_with("; Path=/; Max-Age=60; HttpOnly; Secure; SameSite=Lax"));
        let with = |path: &str| testing::get(path).cookie(COOKIE, &cookie);
        assert_eq!(testing::run_request(&app, with("/me")).json::<User>(), User { name: "ann".into() });
        assert_eq!(testing::run_request(&app, with("/account")).status(), 200);
        assert_eq!(testing::run_request(&app, testing::get("/me").cookie(COOKIE, "not sealed")).text(), "null");
        assert_eq!(testing::run_request(&app, testing::get("/me").sealed_cookie("other", "x")).text(), "null");
        assert_eq!(testing::run_request(&app, testing::get("/me").sealed_cookie(COOKIE, "unknown-id")).text(), "null", "revoked or never stored");

        assert_eq!(testing::run_request(&app, testing::post("/account/rename").cookie(COOKIE, &cookie)).status(), 204);
        assert_eq!(testing::run_request(&app, with("/me")).json::<User>(), User { name: "bob".into() });
        assert_eq!(testing::run_request(&app, testing::post("/account/rename")).status(), 401);

        let out = testing::run_request(&app, testing::delete("/account").cookie(COOKIE, &cookie));
        assert_eq!((out.status(), out.cookie(COOKIE)), (204, None));
        assert_eq!(testing::run_request(&app, with("/account")).status(), 401, "revoked");

        let created = testing::run_request(&app, testing::post("/login"));
        let cookie = created.cookie(COOKIE).unwrap();
        assert_eq!(testing::run_request(&app, testing::get("/account").cookie(COOKIE, &cookie)).status(), 200);
        native::advance(Duration::from_secs(61));
        assert_eq!(testing::run_request(&app, testing::get("/account").cookie(COOKIE, &cookie)).status(), 401, "expired");
    }

    #[test]
    fn a_session_of_another_shape_is_none() {
        native::reset();
        let app = app();
        let created = testing::run_request(&app, testing::post("/login"));
        let cookie = created.cookie(COOKIE).unwrap();
        #[derive(Deserialize)]
        struct Other {
            #[allow(dead_code)]
            id: i64,
        }
        let raw = islas_sdk::Request::new(islas_sdk::Method::Get, "/me").with_header("cookie", format!("{COOKIE}={cookie}"));
        let request = Request::new(raw, Vec::new());
        assert!(get::<Other>(&request).unwrap().is_none());
        assert!(get::<User>(&request).unwrap().is_some());
    }
}
