//! The clock the native backend's TTLs and token expiries run on: the
//! system's, moved forward by [`advance`] so a test sees things expire
//! without sleeping. One per thread, like the stores.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

thread_local! {
    static AHEAD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
}

/// Moves this thread's clock forward by `by`: KV entries and signed tokens
/// that would expire within `by` are expired.
pub fn advance(by: Duration) {
    AHEAD.with(|ahead| ahead.set(ahead.get() + by));
}

/// Puts this thread's clock back on the system's.
pub fn reset() {
    AHEAD.with(|ahead| ahead.set(Duration::ZERO));
}

/// Now on this thread's clock, since the Unix epoch.
pub fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default() + AHEAD.with(Cell::get)
}
