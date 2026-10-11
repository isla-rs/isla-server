//! Write islas apps in Rust: an [`App`] that says what the app has, a
//! router from paths to controllers, requests and responses with JSON in
//! and out, errors that become statuses, pure services, rows read into
//! your types, and tests that run without the host.
//!
//! ```ignore
//! // config/mod.rs: the app's shape
//! use islas::App;
//!
//! mod routes;
//!
//! pub fn app() -> App {
//!     App::new().routes(routes::draw)
//! }
//!
//! // config/routes.rs
//! use islas::web::{Action::*, Routes};
//!
//! pub fn draw(r: &mut Routes) {
//!     r.scope("/api", |r| {
//!         r.resources::<notes::Notes>("notes", [Index, Create]);
//!     });
//! }
//!
//! // src/controllers/notes.rs
//! pub struct Notes;
//!
//! impl Controller for Notes {}
//!
//! impl Resource for Notes {
//!     fn index(_req: Request) -> web::Result {
//!         let notes: Vec<Note> = db::query_as("SELECT id, text FROM notes", &[])?;
//!         Ok(Response::json(&notes))
//!     }
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
//! The platform's imports are the SDK's, re-exported: [`db`] (with
//! [`db::query_as`] added), [`kv`], [`secrets`], [`crypto`], [`fetch`],
//! [`random`], [`http`]. [`web`] is what controllers use (requests,
//! responses, sealed cookies, [`web::session`], [`web::csrf`], pages and
//! files the host sends); [`data`] is where the app keeps its files;
//! [`service`] is what services answer with. With the `native` feature, [`testing`] runs an app against
//! an in-memory database.
//!
//! An app keeps nothing between events: every request gets a fresh instance,
//! which builds the [`App`] again, so building one is cheap by design.

pub mod app;
pub mod data;
pub mod db;
pub mod service;
pub mod web;

#[cfg(all(any(feature = "native", test), not(target_arch = "wasm32")))]
pub mod testing;

pub use app::App;
pub use islas_sdk::{conn, crypto, fetch, http, jobs, kv, random, secrets};

#[doc(hidden)]
pub use islas_sdk as __sdk;
