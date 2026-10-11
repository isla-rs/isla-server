//! Random bytes from the host's cryptographically secure generator
//! (`wasi:random/random`), for ids nobody can guess: sessions, tokens.
//!
//! Built natively (for `cargo test`), they come from `/dev/urandom`.

use std::fmt::Write as _;
use std::io::Read as _;

/// `n` random bytes.
pub fn bytes(n: usize) -> Vec<u8> {
    if cfg!(target_arch = "wasm32") {
        crate::bindings::wasi::random::random::get_random_bytes(n as u64)
    } else {
        let mut bytes = vec![0; n];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .expect("reading /dev/urandom");
        bytes
    }
}

/// 128 random bits as 32 lowercase hex characters.
pub fn token() -> String {
    bytes(16).iter().fold(String::with_capacity(32), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn tokens_are_hex_and_differ() {
        let (a, b) = (super::token(), super::token());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{a}");
        assert_ne!(a, b);
    }
}
