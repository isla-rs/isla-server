//! Sealing and signing under the app's own keys (`platform:app/crypto`),
//! which the host derives from the platform-made `ISLAS_SECRET_KEY`: the
//! app never sees a key and carries no cipher. The framework's sealed
//! cookies, sessions and CSRF tokens are built on these; an app uses them
//! directly for its own tokens.
//!
//! ```ignore
//! use std::time::Duration;
//! use islas_sdk::crypto;
//!
//! // A password-reset link: the user's id, good for an hour.
//! let token = crypto::sign("reset", user_id.as_bytes(), Duration::from_secs(3600))?;
//! // Later, from the link:
//! let user_id = crypto::verify_str("reset", &token).ok_or(Error::Forbidden)?;
//! ```
//!
//! Every call takes a **label**: what the value is for (a cookie's name,
//! `"reset"`, `"unsubscribe"`), 1–256 bytes. A value sealed or signed under
//! one label is nothing under another, so a token made for one purpose
//! can't be replayed for another. Values are at most 4 KiB. The outputs are
//! base64url, fit for a cookie or a URL as they are.
//!
//! Rotating the key (`islas secrets rotate-secret-key`) makes every
//! sealed value and token invalid at once. In native tests (`native`)
//! the keys are random per thread; `native::crypto::rotate` replaces them.

use std::fmt;
use std::time::Duration;

use crate::bindings::platform::app::crypto as host;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An empty label, or one over 256 bytes.
    BadLabel,
    /// A plaintext or value over 4 KiB.
    TooLarge,
    Unavailable(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadLabel => f.write_str("crypto: a label is 1-256 bytes"),
            Error::TooLarge => f.write_str("crypto: a value is at most 4 KiB"),
            Error::Unavailable(message) => write!(f, "crypto: unavailable: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<host::Error> for Error {
    fn from(e: host::Error) -> Self {
        match e {
            host::Error::BadLabel => Error::BadLabel,
            host::Error::TooLarge => Error::TooLarge,
            host::Error::Unavailable(message) => Error::Unavailable(message),
        }
    }
}

/// `plaintext` encrypted and bound to `label`, as a string that only
/// [`open`] with the same label reads.
pub fn seal(label: &str, plaintext: &[u8]) -> Result<String, Error> {
    if cfg!(target_arch = "wasm32") { Ok(host::seal(label, plaintext)?) } else { crate::off_host::crypto::seal(label, plaintext) }
}

/// What [`seal`] made under `label`; `None` when it was changed, sealed
/// under another label or key, or isn't a sealed value.
pub fn open(label: &str, sealed: &str) -> Option<Vec<u8>> {
    if cfg!(target_arch = "wasm32") { host::open(label, sealed) } else { crate::off_host::crypto::open(label, sealed) }
}

/// A token carrying `value`, bound to `label`, that [`verify`] accepts for
/// `ttl` (whole seconds).
pub fn sign(label: &str, value: &[u8], ttl: Duration) -> Result<String, Error> {
    let ttl = ttl.as_secs();
    if cfg!(target_arch = "wasm32") { Ok(host::sign(label, value, ttl)?) } else { crate::off_host::crypto::sign(label, value, ttl) }
}

/// The value in `token` when it was made by [`sign`] under `label` and
/// hasn't expired; `None` otherwise.
pub fn verify(label: &str, token: &str) -> Option<Vec<u8>> {
    if cfg!(target_arch = "wasm32") { host::verify(label, token) } else { crate::off_host::crypto::verify(label, token) }
}

/// [`verify`], for a value that is text (an id); `None` if it isn't UTF-8.
pub fn verify_str(label: &str, token: &str) -> Option<String> {
    String::from_utf8(verify(label, token)?).ok()
}
