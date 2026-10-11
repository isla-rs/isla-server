//! Plain cookies: what `Set-Cookie` says, and reading them back.

use std::time::Duration;

/// A cookie to set, with safe defaults: `Path=/; HttpOnly; Secure;
/// SameSite=Lax`, a session cookie unless [`Cookie::max_age`] is given.
/// Set with [`Response::cookie`](super::Response::cookie) the value is
/// sealed (read back with [`Request::cookie`](super::Request::cookie));
/// with [`Response::plain_cookie`](super::Response::plain_cookie) it is
/// percent-encoded where cookie syntax needs it, and decoded by
/// [`Request::cookie_plain`](super::Request::cookie_plain).
#[derive(Debug, Clone)]
pub struct Cookie {
    name: String,
    value: String,
    max_age: Option<Duration>,
    path: String,
    http_only: bool,
    secure: bool,
    same_site: SameSite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SameSite {
    Strict,
    Lax,
    /// Sent on cross-site requests too (needs `Secure`).
    None,
}

impl Cookie {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Cookie {
            name: name.into(),
            value: value.into(),
            max_age: None,
            path: "/".to_string(),
            http_only: true,
            secure: true,
            same_site: SameSite::Lax,
        }
    }

    /// Kept for this long (whole seconds); without it, until the browser closes.
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.max_age = Some(max_age);
        self
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// `false` for the rare cookie the frontend's JavaScript must read.
    pub fn http_only(mut self, http_only: bool) -> Self {
        self.http_only = http_only;
        self
    }

    pub fn secure(mut self, secure: bool) -> Self {
        self.secure = secure;
        self
    }

    pub fn same_site(mut self, same_site: SameSite) -> Self {
        self.same_site = same_site;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// The `Set-Cookie` value, the cookie's value as it is.
    pub(crate) fn header(&self) -> String {
        self.header_with(&encode(&self.value))
    }

    /// The `Set-Cookie` value with the value sealed under the cookie's
    /// name, so a value moved to another cookie doesn't open.
    pub(crate) fn sealed_header(&self) -> Result<String, islas_sdk::crypto::Error> {
        Ok(self.header_with(&islas_sdk::crypto::seal(&self.name, self.value.as_bytes())?))
    }

    fn header_with(&self, value: &str) -> String {
        let mut header = format!("{}={}; Path={}", self.name, value, self.path);
        if let Some(max_age) = self.max_age {
            header.push_str(&format!("; Max-Age={}", max_age.as_secs()));
        }
        if self.http_only {
            header.push_str("; HttpOnly");
        }
        if self.secure {
            header.push_str("; Secure");
        }
        header.push_str(match self.same_site {
            SameSite::Strict => "; SameSite=Strict",
            SameSite::Lax => "; SameSite=Lax",
            SameSite::None => "; SameSite=None",
        });
        header
    }
}

/// The value of cookie `name` in `Cookie` header values, decoded.
pub(crate) fn find<'a>(headers: impl Iterator<Item = &'a str>, name: &str) -> Option<String> {
    headers
        .flat_map(|header| header.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, value)| super::request::percent_decode(value.trim_matches('"')).unwrap_or_else(|| value.to_string()))
}

/// Percent-encodes what isn't a cookie-octet (RFC 6265), and `%`.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        let octet = matches!(b, 0x21 | 0x23..=0x2B | 0x2D..=0x3A | 0x3C..=0x5B | 0x5D..=0x7E) && b != b'%';
        if octet {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        assert_eq!(Cookie::new("theme", "dark").header(), "theme=dark; Path=/; HttpOnly; Secure; SameSite=Lax");
        let open = Cookie::new("tz", "Europe/Berlin; x").max_age(Duration::from_secs(60)).http_only(false);
        assert_eq!(open.header(), "tz=Europe/Berlin%3B%20x; Path=/; Max-Age=60; Secure; SameSite=Lax");
    }

    #[test]
    fn found_and_decoded() {
        let headers = ["a=1; tz=Europe/Berlin%3B%20x", "b=2"];
        assert_eq!(find(headers.into_iter(), "tz").as_deref(), Some("Europe/Berlin; x"));
        assert_eq!(find(headers.into_iter(), "b").as_deref(), Some("2"));
        assert_eq!(find(headers.into_iter(), "c"), None);
    }
}
