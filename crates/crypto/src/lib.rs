//! What `platform:app/crypto` does (`docs/abi.md` § crypto), as one library
//! used by the runtime on the host and by the SDK's `native` feature in an
//! app's tests, so a cookie sealed in a test opens exactly as one on the
//! server would. It is never part of an app's wasm: the app calls the host
//! and never sees a key.
//!
//! From the app's `ISLAS_SECRET_KEY` two keys are derived with HKDF-SHA256:
//! one for `seal` / `open` (XChaCha20-Poly1305, the plaintext bound to a
//! label such as the cookie's name) and one for `sign` / `verify`
//! (HMAC-SHA256 over the label, an expiry and the value). The outputs are
//! base64url without padding, so they go into a cookie or a URL as they
//! are.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hmac::{Hmac, KeyInit as _, Mac as _};
use rand::TryRng as _;
use secrecy::{ExposeSecret as _, SecretBox};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// A derived key.
pub const KEY_LEN: usize = 32;
/// XChaCha20-Poly1305's nonce.
pub const NONCE_LEN: usize = 24;
/// Poly1305's tag.
const TAG_LEN: usize = 16;
/// HMAC-SHA256's tag.
const MAC_LEN: usize = 32;
/// The expiry in a token: Unix seconds, big-endian.
const EXPIRY_LEN: usize = 8;
/// The longest label, in bytes.
pub const MAX_LABEL: usize = 256;
/// The longest plaintext or value, in bytes.
pub const MAX_VALUE: usize = 4096;
/// The longest sealed value or token looked at, in bytes; anything longer
/// is refused without work.
pub const MAX_TEXT: usize = 8192;

/// HKDF's salt and the two purposes.
const SALT: &[u8] = b"islas";
const SEAL_INFO: &[u8] = b"seal";
const SIGN_INFO: &[u8] = b"sign";

/// A key, zeroized when dropped; `Debug` shows nothing.
pub type Key = SecretBox<[u8; KEY_LEN]>;

/// Why `seal` or `sign` refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// An empty label, or one over `MAX_LABEL` bytes.
    BadLabel,
    /// A plaintext or value over `MAX_VALUE` bytes.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BadLabel => write!(f, "crypto: a label is 1-{MAX_LABEL} bytes"),
            Error::TooLarge => write!(f, "crypto: a value is at most {MAX_VALUE} bytes"),
        }
    }
}

impl std::error::Error for Error {}

/// The two keys derived from an app's secret key.
pub struct Keys {
    seal: Key,
    sign: Key,
}

impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Keys([REDACTED])")
    }
}

impl Keys {
    /// The keys for an app whose `ISLAS_SECRET_KEY` is `secret` (its raw
    /// bytes, any length).
    pub fn derive(secret: &[u8]) -> Keys {
        let prk = hkdf_extract(SALT, secret);
        let keys = Keys { seal: hkdf_expand(&prk, SEAL_INFO), sign: hkdf_expand(&prk, SIGN_INFO) };
        drop(prk);
        keys
    }

    /// The keys for one of the host's own purposes (`hub-cookies`) from a
    /// key of the host's (the hub's master key): HKDF-SHA256 to a key for
    /// `purpose` alone, then as `derive`. Apart from every app's keys and
    /// from the key itself.
    pub fn derive_for(secret: &[u8], purpose: &str) -> Keys {
        let prk = hkdf_extract(SALT, secret);
        let own = hkdf_expand(&prk, purpose.as_bytes());
        drop(prk);
        Keys::derive(own.expose_secret())
    }

    /// Keys nobody else has: for a process without a secret key (`islas
    /// run`) and for tests. What they seal dies with them.
    pub fn random() -> Keys {
        let mut secret = [0u8; KEY_LEN];
        fill_random(&mut secret);
        let keys = Keys::derive(&secret);
        zeroize::Zeroize::zeroize(&mut secret);
        keys
    }

    /// `plaintext` encrypted and bound to `label`:
    /// `base64url(nonce || ciphertext)`.
    pub fn seal(&self, label: &str, plaintext: &[u8]) -> Result<String, Error> {
        check_label(label)?;
        if plaintext.len() > MAX_VALUE {
            return Err(Error::TooLarge);
        }
        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut nonce);
        let cipher = XChaCha20Poly1305::new(self.seal.expose_secret().into());
        let ciphertext = cipher
            .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad: label.as_bytes() })
            .expect("XChaCha20-Poly1305 encrypts any message under the size limit");
        let mut bytes = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        bytes.extend_from_slice(&nonce);
        bytes.extend_from_slice(&ciphertext);
        Ok(B64.encode(bytes))
    }

    /// What `seal` made under the same label and keys; `None` when the text
    /// was changed, sealed under another label or key, or isn't one.
    pub fn open(&self, label: &str, sealed: &str) -> Option<Vec<u8>> {
        if check_label(label).is_err() || sealed.len() > MAX_TEXT {
            return None;
        }
        let bytes = B64.decode(sealed).ok()?;
        if bytes.len() < NONCE_LEN + TAG_LEN {
            return None;
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        let cipher = XChaCha20Poly1305::new(self.seal.expose_secret().into());
        cipher.decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad: label.as_bytes() }).ok()
    }

    /// A token carrying `value`, bound to `label`, valid until `expires_at`
    /// (Unix seconds): `base64url(expiry || value) "." base64url(mac)`.
    pub fn sign(&self, label: &str, value: &[u8], expires_at: u64) -> Result<String, Error> {
        check_label(label)?;
        if value.len() > MAX_VALUE {
            return Err(Error::TooLarge);
        }
        let mut body = Vec::with_capacity(EXPIRY_LEN + value.len());
        body.extend_from_slice(&expires_at.to_be_bytes());
        body.extend_from_slice(value);
        let mac = self.mac(label, &body).finalize().into_bytes();
        Ok(format!("{}.{}", B64.encode(&body), B64.encode(mac)))
    }

    /// The value in `token` when its MAC holds under `label` (compared in
    /// constant time) and its expiry is after `now` (Unix seconds); `None`
    /// otherwise.
    pub fn verify(&self, label: &str, token: &str, now: u64) -> Option<Vec<u8>> {
        if check_label(label).is_err() || token.len() > MAX_TEXT {
            return None;
        }
        let (body, mac) = token.split_once('.')?;
        let body = B64.decode(body).ok()?;
        let mac = B64.decode(mac).ok()?;
        if body.len() < EXPIRY_LEN || mac.len() != MAC_LEN {
            return None;
        }
        self.mac(label, &body).verify_slice(&mac).ok()?;
        let (expiry, value) = body.split_at(EXPIRY_LEN);
        let expires_at = u64::from_be_bytes(expiry.try_into().ok()?);
        (now < expires_at).then(|| value.to_vec())
    }

    /// HMAC over `label || 0 || body`.
    fn mac(&self, label: &str, body: &[u8]) -> HmacSha256 {
        let mut mac = HmacSha256::new_from_slice(self.sign.expose_secret()).expect("HMAC takes a key of any length");
        mac.update(label.as_bytes());
        mac.update(&[0]);
        mac.update(body);
        mac
    }
}

fn check_label(label: &str) -> Result<(), Error> {
    if label.is_empty() || label.len() > MAX_LABEL { Err(Error::BadLabel) } else { Ok(()) }
}

fn fill_random(bytes: &mut [u8]) {
    rand::rngs::SysRng.try_fill_bytes(bytes).expect("the system's random numbers");
}

/// RFC 5869 extract: `HMAC(salt, ikm)`.
fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> Key {
    let mut mac = HmacSha256::new_from_slice(salt).expect("HMAC takes a key of any length");
    mac.update(ikm);
    SecretBox::new(Box::new(mac.finalize().into_bytes().into()))
}

/// RFC 5869 expand for one block (32 bytes): `HMAC(prk, info || 0x01)`.
fn hkdf_expand(prk: &Key, info: &[u8]) -> Key {
    let mut mac = HmacSha256::new_from_slice(prk.expose_secret()).expect("HMAC takes a key of any length");
    mac.update(info);
    mac.update(&[1]);
    SecretBox::new(Box::new(mac.finalize().into_bytes().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Keys {
        Keys::derive(b"a secret key for the tests")
    }

    #[test]
    fn hkdf_matches_rfc_5869_case_1() {
        // Test case 1, with L cut to the first 32 bytes of the 42.
        let ikm = [0x0b; 22];
        let salt: Vec<u8> = (0x00..=0x0c).collect();
        let info: Vec<u8> = (0xf0..=0xf9).collect();
        let prk = hkdf_extract(&salt, &ikm);
        assert_eq!(
            hex(prk.expose_secret()),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        let okm = hkdf_expand(&prk, &info);
        assert_eq!(
            hex(okm.expose_secret()),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf"
        );
    }

    #[test]
    fn sealed_values_open_only_in_their_place() {
        let k = keys();
        let sealed = k.seal("session", b"ann").unwrap();
        assert!(!sealed.contains('=') && !sealed.contains('+') && !sealed.contains('/'), "{sealed}");
        assert_ne!(sealed, k.seal("session", b"ann").unwrap(), "a fresh nonce every time");
        assert_eq!(k.open("session", &sealed).as_deref(), Some(&b"ann"[..]));
        assert_eq!(k.open("other", &sealed), None, "another label");
        assert_eq!(keys_from(b"other").open("session", &sealed), None, "another key");
        assert_eq!(Keys::random().open("session", &sealed), None);
        let mut damaged = sealed.clone();
        let last = damaged.pop().unwrap();
        damaged.push(if last == 'A' { 'B' } else { 'A' });
        assert_eq!(k.open("session", &damaged), None, "damaged");
        assert_eq!(k.open("session", "not base64!"), None);
        assert_eq!(k.open("session", "AAAA"), None, "too short");
        assert_eq!(k.open("session", &"A".repeat(MAX_TEXT + 1)), None, "too long");
        assert_eq!(k.open("", &sealed), None);
        assert_eq!(k.seal("", b"x"), Err(Error::BadLabel));
        assert_eq!(k.seal(&"l".repeat(MAX_LABEL + 1), b"x"), Err(Error::BadLabel));
        assert_eq!(k.seal("x", &[0; MAX_VALUE + 1]), Err(Error::TooLarge));
        assert!(k.seal("x", &[0; MAX_VALUE]).is_ok());
        assert_eq!(k.open("x", &k.seal("x", b"").unwrap()).as_deref(), Some(&b""[..]), "an empty value");
    }

    #[test]
    fn tokens_expire_and_bind_to_their_label() {
        let k = keys();
        let token = k.sign("reset", b"42", 1000).unwrap();
        assert_eq!(k.verify("reset", &token, 999).as_deref(), Some(&b"42"[..]));
        assert_eq!(k.verify("reset", &token, 1000), None, "expired at the second");
        assert_eq!(k.verify("unsubscribe", &token, 1), None, "another label");
        assert_eq!(keys_from(b"other").verify("reset", &token, 1), None, "another key");
        let (body, mac) = token.split_once('.').unwrap();
        assert_eq!(k.verify("reset", &format!("{body}.{}", B64.encode([0u8; 32])), 1), None, "a wrong mac");
        let other_body = B64.encode([&u64::MAX.to_be_bytes()[..], b"42"].concat());
        assert_eq!(k.verify("reset", &format!("{other_body}.{mac}"), 1), None, "a changed expiry");
        assert_eq!(k.verify("reset", body, 1), None, "no mac");
        assert_eq!(k.verify("reset", "a.b", 1), None);
        assert_eq!(k.verify("reset", "", 1), None);
        assert_eq!(k.sign("reset", &[0; MAX_VALUE + 1], 1), Err(Error::TooLarge));
    }

    #[test]
    fn a_token_is_not_a_cookie_and_the_other_way() {
        let k = keys();
        let token = k.sign("x", b"v", u64::MAX).unwrap();
        assert_eq!(k.open("x", &token), None);
        let sealed = k.seal("x", b"v").unwrap();
        assert_eq!(k.verify("x", &sealed, 0), None);
        assert_eq!(format!("{k:?}"), "Keys([REDACTED])");
    }

    fn keys_from(secret: &[u8]) -> Keys {
        Keys::derive(secret)
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
