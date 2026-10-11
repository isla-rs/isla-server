//! `platform:app/crypto` in memory: the host's own primitives
//! (`islas_crypto`) under random keys, one set per thread, so what a test
//! seals opens exactly as it would on the server. [`rotate`] replaces the
//! keys, as `islas secrets rotate-secret-key` does.

use std::cell::RefCell;

use islas_crypto::Keys;

use crate::crypto::Error;

thread_local! {
    static KEYS: RefCell<Keys> = RefCell::new(Keys::random());
}

/// New keys for this thread: everything sealed or signed before is
/// invalid now.
pub fn rotate() {
    KEYS.with_borrow_mut(|keys| *keys = Keys::random());
}

/// `rotate`, for `native::reset`.
pub fn reset() {
    rotate();
}

fn convert(e: islas_crypto::Error) -> Error {
    match e {
        islas_crypto::Error::BadLabel => Error::BadLabel,
        islas_crypto::Error::TooLarge => Error::TooLarge,
    }
}

pub(crate) fn seal(label: &str, plaintext: &[u8]) -> Result<String, Error> {
    KEYS.with_borrow(|keys| keys.seal(label, plaintext)).map_err(convert)
}

pub(crate) fn open(label: &str, sealed: &str) -> Option<Vec<u8>> {
    KEYS.with_borrow(|keys| keys.open(label, sealed))
}

pub(crate) fn sign(label: &str, value: &[u8], ttl_seconds: u64) -> Result<String, Error> {
    let expires_at = super::clock::now().as_secs().saturating_add(ttl_seconds);
    KEYS.with_borrow(|keys| keys.sign(label, value, expires_at)).map_err(convert)
}

pub(crate) fn verify(label: &str, token: &str) -> Option<Vec<u8>> {
    KEYS.with_borrow(|keys| keys.verify(label, token, super::clock::now().as_secs()))
}
