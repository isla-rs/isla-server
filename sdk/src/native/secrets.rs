//! `platform:app/secrets` from a map, one per thread.

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static SECRETS: RefCell<HashMap<String, String>> = RefCell::default();
}

/// Sets a secret for this thread, as `islas secrets set` would.
pub fn set(name: &str, value: &str) {
    SECRETS.with_borrow_mut(|secrets| secrets.insert(name.to_string(), value.to_string()));
}

/// Removes a secret for this thread.
pub fn remove(name: &str) {
    SECRETS.with_borrow_mut(|secrets| secrets.remove(name));
}

/// Removes every secret for this thread.
pub fn reset() {
    SECRETS.with_borrow_mut(HashMap::clear);
}

pub(crate) fn get(name: &str) -> Option<String> {
    SECRETS.with_borrow(|secrets| secrets.get(name).cloned())
}
