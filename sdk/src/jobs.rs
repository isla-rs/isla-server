//! The app's job queue (`platform:app/jobs`) and the job handler. The host
//! runs jobs from E2; until then `enqueue` answers `unavailable`.

use crate::bindings::platform::app::jobs as host;

pub use crate::bindings::job::exports::platform::app::job_handler::{
    Guest as JobHandler, Outcome,
};
pub use crate::bindings::platform::app::types::Timestamp;
pub use host::Error;

/// Queues a job and returns its id; `run_at` (ms since the epoch) absent
/// means as soon as possible.
pub fn enqueue(name: &str, payload: &[u8], run_at: Option<Timestamp>) -> Result<u64, Error> {
    if cfg!(target_arch = "wasm32") {
        host::enqueue(name, payload, run_at)
    } else {
        Err(Error::Unavailable(crate::NATIVE.to_string()))
    }
}
