//! The app's secrets (`platform:app/secrets`). Never log a value.

use crate::bindings::platform::app::secrets as host;

/// The secret's value, or `None` when it isn't set.
pub fn get(name: &str) -> Option<String> {
    if cfg!(target_arch = "wasm32") { host::get(name) } else { crate::off_host::secrets::get(name) }
}
