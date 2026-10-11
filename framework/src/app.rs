//! The app as it describes itself. `config/mod.rs` builds an [`App`] that
//! names what the app has (its routes; jobs and the other handler kinds
//! come later) and [`app!`] exports it: the framework turns on what the
//! `App` declares when the app starts and serves its router on every
//! request. No app calls `http::listen`, writes a `start` or names a
//! handler kind.
//!
//! ```ignore
//! // config/mod.rs
//! use islas::App;
//!
//! mod routes;
//!
//! pub fn app() -> App {
//!     App::new().routes(routes::draw)
//! }
//!
//! // src/lib.rs
//! #[path = "../config/mod.rs"]
//! mod config;
//! mod controllers;
//! mod models;
//! mod services;
//!
//! islas::app!(config::app);
//! ```
//!
//! `app()` runs once per event: an app keeps nothing between events, so
//! there is nothing else to initialize, and building an `App` is cheap by
//! design.

use std::fmt;

use islas_sdk::http;

use crate::web::{Router, Routes};

/// What the app has. An `App` that declares nothing fails to start: the
/// platform would have nothing to turn on.
#[derive(Default)]
pub struct App {
    router: Router,
    http: bool,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// The routes every HTTP request goes through, drawn by `draw`
    /// (`config/routes.rs`). With them the app serves HTTP.
    pub fn routes(mut self, draw: impl FnOnce(&mut Routes)) -> Self {
        self.router = Routes::draw(draw);
        self.http = true;
        self
    }

    /// The router from [`App::routes`], or an empty one that answers 404.
    pub fn router(&self) -> &Router {
        &self.router
    }

    /// Turns on what the app declares. Called once, when the app starts,
    /// by the code [`app!`] generates.
    #[doc(hidden)]
    pub fn start(&self) -> Result<(), StartError> {
        if !self.http {
            return Err(StartError::Nothing);
        }
        http::listen().map_err(StartError::Http)
    }
}

/// Why the app didn't start.
#[derive(Debug)]
pub enum StartError {
    /// The `App` declares nothing: no routes.
    Nothing,
    /// The host refused to open the HTTP listener.
    Http(http::Error),
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartError::Nothing => write!(f, "the App declares nothing: give it routes"),
            StartError::Http(e) => write!(f, "listening for HTTP: {e}"),
        }
    }
}

impl std::error::Error for StartError {}

/// Exports the app: a `fn() -> App`, by path. At start the `App` is built
/// and what it declares is turned on; every HTTP request is served by its
/// router.
///
/// ```ignore
/// islas::app!(config::app);
/// ```
#[macro_export]
macro_rules! app {
    ($app:path) => {
        #[doc(hidden)]
        fn __islas_start() -> ::core::result::Result<(), $crate::app::StartError> {
            $app().start()
        }

        #[doc(hidden)]
        fn __islas_http(request: $crate::__sdk::Request) -> $crate::__sdk::Response {
            $crate::web::serve($app().router(), request)
        }

        // The exports only exist in the component; natively (tests) they
        // would be undefined symbols, so the items are only referenced.
        #[cfg(target_arch = "wasm32")]
        $crate::__sdk::app!(__islas_start, http = __islas_http);

        #[cfg(not(target_arch = "wasm32"))]
        #[doc(hidden)]
        #[allow(dead_code)]
        pub fn __islas_native_entry() {
            let _ = (__islas_start as fn() -> _, __islas_http as fn(_) -> _);
        }
    };
}

#[cfg(test)]
mod tests {
    use islas_sdk::{Method, Request};

    use super::*;
    use crate::web::{self, Response};

    fn hello(_req: web::Request) -> web::Result {
        Ok(Response::text("hi"))
    }

    fn app() -> App {
        App::new().routes(|r| {
            r.get("/hello", hello);
        })
    }

    crate::app!(app);

    #[test]
    fn starts_with_routes_and_not_without() {
        assert!(matches!(App::new().start(), Err(StartError::Nothing)));
        assert_eq!(App::new().start().unwrap_err().to_string(), "the App declares nothing: give it routes");
        assert!(__islas_start().is_ok(), "natively, listen always succeeds");
    }

    #[test]
    fn the_export_serves_the_router() {
        let res = __islas_http(Request::new(Method::Get, "/hello"));
        assert_eq!((res.status(), res.body()), (200, b"hi".as_slice()));
        assert_eq!(__islas_http(Request::new(Method::Get, "/nothing")).status(), 404);
        assert_eq!(App::new().router().handle(Request::new(Method::Get, "/hello")).status_code(), 404);
    }
}
