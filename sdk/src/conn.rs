//! The app's open connections (`platform:app/conn`) and their handlers. The
//! host holds WebSocket connections from E3 and TCP ones from E4; until then
//! these answer `unavailable`.

use crate::bindings::platform::app::conn as host;

pub use crate::bindings::platform::app::types::{ConnId, Message};
pub use crate::bindings::stream::exports::platform::app::stream_handler::Guest as StreamHandler;
pub use crate::bindings::ws::exports::platform::app::ws_handler::Guest as WsHandler;
pub use host::Error;

/// Sends a message; on a TCP connection text goes as its UTF-8 bytes.
pub fn send(conn: ConnId, message: impl Into<Message>) -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") { host::send(conn, &message.into()) } else { Err(native()) }
}

pub fn close(conn: ConnId) -> Result<(), Error> {
    if cfg!(target_arch = "wasm32") { host::close(conn) } else { Err(native()) }
}

/// The app's open connections.
pub fn list() -> Vec<ConnId> {
    if cfg!(target_arch = "wasm32") { host::list() } else { Vec::new() }
}

impl From<String> for Message {
    fn from(text: String) -> Self {
        Message::Text(text)
    }
}

impl From<&str> for Message {
    fn from(text: &str) -> Self {
        Message::Text(text.to_string())
    }
}

impl From<Vec<u8>> for Message {
    fn from(bytes: Vec<u8>) -> Self {
        Message::Binary(bytes)
    }
}

impl From<&[u8]> for Message {
    fn from(bytes: &[u8]) -> Self {
        Message::Binary(bytes.to_vec())
    }
}

fn native() -> Error {
    Error::Unavailable(crate::NATIVE.to_string())
}
