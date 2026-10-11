//! The imports run natively, for tests (feature `native`, never in wasm):
//! [`db`] on an in-memory SQLite, [`kv`] in a map with TTLs, [`secrets`]
//! from a map, [`crypto`] with the host's own primitives under random
//! keys, all on a [`clock`] a test can move forward. Each thread has its
//! own, so tests running in parallel never see each other's rows; a test
//! starts empty.
//!
//! The functions here set a test up (a schema, a secret, the clock); the
//! app's code calls the usual `islas_sdk::db`, `kv`, `secrets` and
//! `crypto`.

pub mod clock;
pub mod crypto;
pub mod db;
pub mod kv;
pub mod secrets;

pub use clock::advance;

/// Empties this thread's database, store and secrets, makes new crypto
/// keys and puts the clock back.
pub fn reset() {
    db::reset();
    kv::reset();
    secrets::reset();
    crypto::reset();
    clock::reset();
}
