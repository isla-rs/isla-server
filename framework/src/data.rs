//! The app's `/data`: the one folder it can write, private to it (the
//! database, KV and secrets live elsewhere). Files are written with
//! `std::fs` at [`path`] and answered with
//! [`Response::data_file`](crate::web::Response::data_file) or
//! [`Response::download`](crate::web::Response::download), which the host
//! sends. `/data/public/` is served to anyone at `/public` when the
//! manifest says `[http] public = true`; everything else stays private.
//! Natively (tests) it is a fresh temporary folder per thread, or what
//! `testing::with_data` names.

use std::path::PathBuf;

use crate::web::files::Folder;

/// Where `relative` (`exports/notes.csv`) is under `/data`.
pub fn path(relative: &str) -> PathBuf {
    Folder::Data.root().join(relative.trim_start_matches('/'))
}
