//! `platform:app/kv` in a map, one store per thread, with the host's rules
//! (`docs/abi.md` § kv): keys of 1–1024 bytes without whitespace or control
//! characters, values up to 1 MiB, 64 MiB in all, a TTL of 0 deletes,
//! pages of at most 1000 entries and about 1 MiB. Its clock is the
//! backend's ([`super::clock`]), moved forward with [`advance`] so a test
//! sees a TTL expire without sleeping.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Bound;
use std::time::Duration;

use crate::bindings::platform::app::kv::{Entry, Error};

const MAX_KEY: usize = 1024;
const MAX_VALUE: usize = 1 << 20;
const QUOTA: usize = 64 << 20;
const MAX_LIST: usize = 1000;
const MAX_PAGE_BYTES: usize = 1 << 20;

#[derive(Default)]
struct Store {
    /// Key → value and when it expires (ms on the store's clock).
    entries: BTreeMap<String, (Vec<u8>, Option<u64>)>,
    /// Bytes of keys and values held.
    bytes: usize,
}

thread_local! {
    static STORE: RefCell<Store> = RefCell::default();
}

/// Moves this thread's clock forward by `by` (the same as
/// [`super::clock::advance`]).
pub fn advance(by: Duration) {
    super::clock::advance(by);
}

/// Empties this thread's store.
pub fn reset() {
    STORE.with_borrow_mut(|store| *store = Store::default());
}

pub(crate) fn get(key: &str) -> Result<Option<Vec<u8>>, Error> {
    check_key(key, false)?;
    STORE.with_borrow_mut(|store| {
        store.expire(key);
        Ok(store.entries.get(key).map(|(value, _)| value.clone()))
    })
}

pub(crate) fn set(key: &str, value: &[u8], ttl_seconds: Option<u64>) -> Result<(), Error> {
    check_key(key, false)?;
    if value.len() > MAX_VALUE {
        return Err(Error::TooLarge);
    }
    STORE.with_borrow_mut(|store| {
        store.expire(key);
        if ttl_seconds == Some(0) {
            store.remove(key);
            return Ok(());
        }
        let old = store.entries.get(key).map_or(0, |(v, _)| key.len() + v.len());
        if store.bytes - old + key.len() + value.len() > QUOTA {
            return Err(Error::QuotaExceeded);
        }
        let expires = ttl_seconds.map(|s| store.now_ms().saturating_add(s.saturating_mul(1000)));
        store.bytes = store.bytes - old + key.len() + value.len();
        store.entries.insert(key.to_string(), (value.to_vec(), expires));
        Ok(())
    })
}

pub(crate) fn delete(key: &str) -> Result<(), Error> {
    check_key(key, false)?;
    STORE.with_borrow_mut(|store| store.remove(key));
    Ok(())
}

pub(crate) fn list(prefix: &str, start_after: Option<&str>, limit: u32) -> Result<Vec<Entry>, Error> {
    check_key(prefix, true)?;
    if let Some(after) = start_after {
        check_key(after, true)?;
    }
    STORE.with_borrow(|store| {
        let now = store.now_ms();
        let from = match start_after {
            Some(after) if after >= prefix => Bound::Excluded(after),
            _ => Bound::Included(prefix),
        };
        let mut page = Vec::new();
        let mut bytes = 0;
        for (key, (value, expires)) in store.entries.range::<str, _>((from, Bound::Unbounded)) {
            if !key.starts_with(prefix) || page.len() == (limit as usize).min(MAX_LIST) {
                break;
            }
            if expires.is_some_and(|at| at <= now) {
                continue;
            }
            bytes += key.len() + value.len();
            if bytes > MAX_PAGE_BYTES && !page.is_empty() {
                break;
            }
            page.push(Entry { key: key.clone(), value: value.clone() });
        }
        Ok(page)
    })
}

impl Store {
    fn now_ms(&self) -> u64 {
        let now = super::clock::now();
        now.as_millis() as u64
    }

    /// Drops `key` if it expired.
    fn expire(&mut self, key: &str) {
        let now = self.now_ms();
        if self.entries.get(key).is_some_and(|(_, expires)| expires.is_some_and(|at| at <= now)) {
            self.remove(key);
        }
    }

    fn remove(&mut self, key: &str) {
        if let Some((value, _)) = self.entries.remove(key) {
            self.bytes -= key.len() + value.len();
        }
    }
}

/// A key (or, with `empty`, a prefix or `start-after`), as the host checks it.
fn check_key(key: &str, empty: bool) -> Result<(), Error> {
    let ok = (empty || !key.is_empty())
        && key.len() <= MAX_KEY
        && !key.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok { Ok(()) } else { Err(Error::BadKey) }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::kv;

    #[test]
    fn values_expire_on_the_store_clock() {
        kv::set_ttl("session/a", b"ann", Duration::from_secs(60)).unwrap();
        kv::set("keep", b"1").unwrap();
        assert_eq!(kv::get("session/a").unwrap().as_deref(), Some(&b"ann"[..]));
        super::advance(Duration::from_secs(61));
        assert_eq!(kv::get("session/a").unwrap(), None);
        assert_eq!(kv::list("").unwrap().len(), 1, "expired entries aren't listed");
        assert_eq!(kv::get("keep").unwrap().as_deref(), Some(&b"1"[..]));
    }

    #[test]
    fn the_hosts_rules() {
        assert_eq!(kv::get("a b"), Err(kv::Error::BadKey));
        assert_eq!(kv::set("", b"x"), Err(kv::Error::BadKey));
        assert_eq!(kv::set("big", &vec![0; (1 << 20) + 1]), Err(kv::Error::TooLarge));
        kv::set("gone", b"x").unwrap();
        kv::set_ttl("gone", b"x", Duration::ZERO).unwrap();
        assert_eq!(kv::get("gone").unwrap(), None, "a TTL of 0 deletes");
        assert_eq!(kv::delete("never-there"), Ok(()));
    }

    #[test]
    fn pages_in_key_order() {
        for i in 0..1500 {
            kv::set(&format!("n/{i:04}"), b"").unwrap();
        }
        kv::set("other", b"").unwrap();
        let page = kv::list_page("n/", None, 5000).unwrap();
        assert_eq!((page.len(), page[0].key.as_str()), (1000, "n/0000"));
        let next = kv::list_page("n/", Some(&page[999].key), 5000).unwrap();
        assert_eq!((next.len(), next[0].key.as_str()), (500, "n/1000"));
        assert_eq!(kv::list("n/").unwrap().len(), 1500);
    }
}
