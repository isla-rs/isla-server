//! Handlers: the routes `config/routes.rs` draws into a [`Routes`] table.
//! A controller is a type implementing [`Controller`] (`before`, for auth)
//! whose actions are `fn(Request) -> Result<Response, Error>`; one with
//! Rails' actions implements [`Resource`] too.
//!
//! ```ignore
//! pub fn draw(r: &mut Routes) {
//!     r.scope("/api", |r| {
//!         r.resources::<notes::Notes>("notes", [Index, Show, Create, Update, Destroy]);
//!         r.controller::<accounts::Accounts>(|c| {
//!             c.post("transfer", Accounts::transfer);
//!         });
//!     });
//! }
//! ```
//!
//! A failed request is an [`Error`]: its status and `{"error": "…"}`; an
//! internal one is logged and its details never sent.
//!
//! Cookies are **sealed** by default ([`Request::cookie`],
//! [`Response::cookie`]): the browser carries a value it can't read or
//! change, under the app's own key, so a cookie is a safe place for an id
//! or a small setting. A [`session`] is a sealed cookie with its data in
//! KV; [`csrf`] protects forms. [`Response::file`] and [`Response::page`]
//! serve a file of the version's `static/` from a route: a **public page
//! is a file** in `static/` (the host serves it, with the fallback to the
//! nearest `index.html`), a **gated page is a route** whose controller's
//! `before` checks the session. [`Response::data_file`] and
//! [`Response::download`] do the same for a file the app wrote under
//! `/data` (at [`crate::data::path`]). Either way the response only names
//! the file and the host sends it: no bytes pass through the app, and a
//! file may be larger than a response body can be.

mod cookie;
pub mod csrf;
mod error;
pub(crate) mod files;
mod request;
mod response;
mod router;
mod routes;
pub mod session;

pub use cookie::{Cookie, SameSite};
pub use error::Error;
pub use islas_sdk::Method;
pub use request::Request;
pub use response::Response;
pub use router::{Handler, Hook, Router};
pub use routes::{Action, Controller, Resource, Routes};

/// What a controller returns: `Result<Response, Error>` by default.
pub type Result<T = Response, E = Error> = std::result::Result<T, E>;

/// Runs one HTTP event through `router`. Called by the code `app!`
/// generates.
#[doc(hidden)]
pub fn serve(router: &Router, request: islas_sdk::Request) -> islas_sdk::Response {
    router.handle(request).into()
}

/// The method as it is written on the wire.
pub(crate) fn method_name(method: &Method) -> &str {
    match method {
        Method::Get => "GET",
        Method::Head => "HEAD",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
        Method::Connect => "CONNECT",
        Method::Options => "OPTIONS",
        Method::Trace => "TRACE",
        Method::Patch => "PATCH",
        Method::Other(other) => other,
    }
}
