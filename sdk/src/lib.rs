//! Write islas apps in Rust.
//!
//! An app keeps nothing between events: each event runs on a fresh instance.
//! When the app starts, the host calls its `start` function once; by default
//! nothing runs, and `start` turns on what the app needs ([`http::listen`]).
//! The HTTP handler is a function from [`Request`] to [`Response`]; [`app!`]
//! exports `start` and the handlers, and the platform's state is reached
//! through [`db`], [`kv`], [`secrets`], [`jobs`] and [`conn`]; [`fetch`]
//! calls other services; [`random`] makes ids nobody can guess; [`crypto`]
//! seals and signs under keys the app never sees.
//!
//! ```ignore
//! use islas_sdk::{Request, Response, http};
//!
//! fn start() -> Result<(), http::Error> {
//!     http::listen()
//! }
//!
//! fn handle(req: Request) -> Response {
//!     Response::text("hello")
//! }
//!
//! islas_sdk::app!(start, http = handle);
//! ```
//!
//! Built natively (for `cargo test`), the imports are stubs that answer
//! `unavailable` (or nothing, for `secrets` and `conn::list`); with the
//! `native` feature, `db`, `kv`, `secrets` and `crypto` run in memory
//! instead ([`native`]), so an app's tests run without the host.

#[doc(hidden)]
pub mod bindings;

pub mod conn;
pub mod crypto;
pub mod db;
pub mod fetch;
pub mod http;
pub mod jobs;
pub mod kv;
pub mod random;
pub mod secrets;

#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
pub mod native;
/// Where the imports go off the host: in memory with `native`, stubs without.
#[cfg(all(feature = "native", not(target_arch = "wasm32")))]
use native as off_host;
#[cfg(not(all(feature = "native", not(target_arch = "wasm32"))))]
mod off_host;

pub use http::{Method, Request, Response};

/// What the native stubs answer.
const NATIVE: &str = "not running on the islas host";

/// Runs the app's `start` function for the export `app!` generates.
#[doc(hidden)]
pub fn run_start<E: std::fmt::Display>(start: impl FnOnce() -> Result<(), E>) -> Result<(), String> {
    start().map_err(|e| e.to_string())
}

/// Exports the app's `start` function and its handlers. `start` is a
/// `fn() -> Result<(), E>` with `E: Display`, called once when the app
/// starts; the handlers are optional, by kind:
///
/// ```ignore
/// islas_sdk::app!(start);                                  // nothing to handle yet
/// islas_sdk::app!(start, http = handle);
/// islas_sdk::app!(start, http = handle, ws = Chat, jobs = Worker);
/// ```
///
/// `http` takes a `fn(Request) -> Response`, `ws` a type implementing
/// [`conn::WsHandler`], `stream` one implementing [`conn::StreamHandler`],
/// `jobs` one implementing [`jobs::JobHandler`]. Only the handlers named here
/// are exported; having a handler turns nothing on, `start` does.
#[macro_export]
macro_rules! app {
    ($start:ident $(, $kind:ident = $handler:ident)* $(,)?) => {
        #[doc(hidden)]
        pub struct __IslasStart;

        impl $crate::bindings::exports::platform::app::start_handler::Guest for __IslasStart {
            fn start() -> ::core::result::Result<(), ::std::string::String> {
                $crate::run_start($start)
            }
        }

        $crate::bindings::export_start!(__IslasStart with_types_in $crate::bindings);
        $($crate::app!(@export $kind $handler);)*
    };
    (@export http $handler:ident) => {
        #[doc(hidden)]
        pub struct __IslasHttp;

        impl $crate::bindings::http::exports::wasi::http::incoming_handler::Guest for __IslasHttp {
            fn handle(
                request: $crate::bindings::wasi::http::types::IncomingRequest,
                out: $crate::bindings::wasi::http::types::ResponseOutparam,
            ) {
                $crate::http::serve(request, out, $handler)
            }
        }

        $crate::bindings::http::export_http!(__IslasHttp with_types_in $crate::bindings::http);
    };
    (@export ws $ty:ident) => {
        $crate::bindings::ws::export_ws!($ty with_types_in $crate::bindings::ws);
    };
    (@export stream $ty:ident) => {
        $crate::bindings::stream::export_stream!($ty with_types_in $crate::bindings::stream);
    };
    (@export jobs $ty:ident) => {
        $crate::bindings::job::export_job!($ty with_types_in $crate::bindings::job);
    };
}
