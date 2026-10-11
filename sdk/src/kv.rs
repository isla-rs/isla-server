//! Small state by key, with optional expiry (`platform:app/kv`): sessions,
//! caches, counters, settings. The app's own store; no other app sees it.
//!
//! ```ignore
//! use std::time::Duration;
//! use islas_sdk::kv;
//!
//! let visits = kv::get_json::<u64>("visits")?.unwrap_or(0) + 1;
//! kv::set_json("visits", &visits)?;
//! kv::set_json_ttl(&format!("session/{id}"), &session, Duration::from_secs(3600))?;
//! for entry in kv::list("session/")? {
//!     let session: Session = entry.json()?;
//! }
//! ```
//!
//! A key is 1–1024 bytes of UTF-8 without whitespace or control characters
//! ([`Error::BadKey`]); a value at most 1 MiB ([`Error::TooLarge`]); the
//! store holds 64 MiB of keys and values ([`Error::QuotaExceeded`], nothing
//! written). Each write is its own commit, durable within about a second;
//! it is **not** part of a [`db`](crate::db) transaction, so data that has
//! to change together with rows belongs in the database.
//!
//! The `_json` functions store serde types as JSON, which keeps field
//! names: a type can grow, and a field added with `#[serde(default)]` still
//! reads values stored before.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::bindings::platform::app::kv as host;
use crate::off_host;

pub use host::Entry;

/// Entries per page asked of the host (its most).
const PAGE: u32 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Empty, over 1 KiB, or with whitespace or control characters.
    BadKey,
    /// A value over 1 MiB.
    TooLarge,
    /// The store would go past its 64 MiB; nothing was written.
    QuotaExceeded,
    Unavailable(String),
    /// A value that isn't JSON of the type asked for, or a type JSON can't
    /// hold (a map with keys that aren't strings).
    Json(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::BadKey => f.write_str("kv: bad key (1-1024 bytes, no whitespace or control characters)"),
            Error::TooLarge => f.write_str("kv: value over 1 MiB"),
            Error::QuotaExceeded => f.write_str("kv: the store is full"),
            Error::Unavailable(message) => write!(f, "kv: unavailable: {message}"),
            Error::Json(message) => write!(f, "kv: JSON: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<host::Error> for Error {
    fn from(e: host::Error) -> Self {
        match e {
            host::Error::BadKey => Error::BadKey,
            host::Error::TooLarge => Error::TooLarge,
            host::Error::QuotaExceeded => Error::QuotaExceeded,
            host::Error::Unavailable(message) => Error::Unavailable(message),
        }
    }
}

/// The value of `key`; `None` if there is none or it expired.
pub fn get(key: &str) -> Result<Option<Vec<u8>>, Error> {
    if cfg!(target_arch = "wasm32") { Ok(host::get(key)?) } else { Ok(off_host::kv::get(key)?) }
}

/// The value of `key` as a `T`, read from JSON.
pub fn get_json<T: DeserializeOwned>(key: &str) -> Result<Option<T>, Error> {
    get(key)?.map(|bytes| from_json(&bytes)).transpose()
}

/// Sets `key` to `value`, with no expiry.
pub fn set(key: &str, value: &[u8]) -> Result<(), Error> {
    set_raw(key, value, None)
}

/// Sets `key` to `value` for `ttl` (rounded up to whole seconds); after
/// that it is absent. A zero `ttl` deletes the key.
pub fn set_ttl(key: &str, value: &[u8], ttl: Duration) -> Result<(), Error> {
    set_raw(key, value, Some(seconds(ttl)))
}

/// Sets `key` to `value` as JSON, with no expiry.
pub fn set_json<T: Serialize + ?Sized>(key: &str, value: &T) -> Result<(), Error> {
    set(key, &to_json(value)?)
}

/// Sets `key` to `value` as JSON for `ttl` (see [`set_ttl`]).
pub fn set_json_ttl<T: Serialize + ?Sized>(key: &str, value: &T, ttl: Duration) -> Result<(), Error> {
    set_ttl(key, &to_json(value)?, ttl)
}

/// Deletes `key`; deleting a key that isn't there is fine.
pub fn delete(key: &str) -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") { Ok(host::delete(key)?) } else { Ok(off_host::kv::delete(key)?) }
}

/// Every entry whose key starts with `prefix` (`""`: all), in key order,
/// read page by page. It reads them all: keep what one prefix holds
/// reasonable, or use [`list_page`].
pub fn list(prefix: &str) -> Result<Vec<Entry>, Error> {
    let mut entries: Vec<Entry> = Vec::new();
    loop {
        let after = entries.last().map(|e| e.key.clone());
        let page = list_page(prefix, after.as_deref(), PAGE)?;
        if page.is_empty() {
            return Ok(entries);
        }
        entries.extend(page);
    }
}

/// One page of entries whose key starts with `prefix`, in key order, after
/// `start_after`: at most `limit` (1000 at most) and about 1 MiB of keys and
/// values (always at least one entry if there is one). The next page starts
/// after the last key; an empty page is the end.
pub fn list_page(prefix: &str, start_after: Option<&str>, limit: u32) -> Result<Vec<Entry>, Error> {
    if cfg!(target_arch = "wasm32") {
        Ok(host::list(prefix, start_after, limit)?)
    } else {
        Ok(off_host::kv::list(prefix, start_after, limit)?)
    }
}

impl Entry {
    /// The value as a `T`, read from JSON.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, Error> {
        from_json(&self.value)
    }
}

fn set_raw(key: &str, value: &[u8], ttl_seconds: Option<u64>) -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") {
        Ok(host::set(key, value, ttl_seconds)?)
    } else {
        Ok(off_host::kv::set(key, value, ttl_seconds)?)
    }
}

/// Whole seconds, a part of one counting as one.
fn seconds(ttl: Duration) -> u64 {
    ttl.as_secs() + u64::from(ttl.subsec_nanos() > 0)
}

fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(value).map_err(|e| Error::Json(e.to_string()))
}

fn from_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|e| Error::Json(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ttl_is_whole_seconds_rounded_up() {
        assert_eq!(seconds(Duration::ZERO), 0);
        assert_eq!(seconds(Duration::from_millis(1)), 1);
        assert_eq!(seconds(Duration::from_secs(2)), 2);
        assert_eq!(seconds(Duration::from_millis(2001)), 3);
    }

    #[test]
    fn json_values_can_grow_a_field() {
        #[derive(serde::Serialize)]
        struct Old {
            user: String,
        }
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct New {
            user: String,
            #[serde(default)]
            admin: bool,
        }
        let stored = to_json(&Old { user: "ann".into() }).unwrap();
        assert_eq!(from_json::<New>(&stored), Ok(New { user: "ann".into(), admin: false }));
        assert!(matches!(from_json::<u64>(b"\"text\""), Err(Error::Json(_))));
    }

    #[test]
    #[cfg(not(feature = "native"))]
    fn natively_the_store_is_unavailable() {
        assert!(matches!(get_json::<u64>("k"), Err(Error::Unavailable(_))));
        assert!(matches!(list("p"), Err(Error::Unavailable(_))));
    }
}
