//! What the imports answer off the host without the `native` feature: a
//! native build that only has to compile (`cargo check`, `clippy`, unit
//! tests that don't touch the platform).

const NATIVE: &str = "not running on the islas host (the `native` feature runs the imports in memory)";

pub mod db {
    use crate::db::{Error, Rows, Value};

    pub fn query(_sql: &str, _params: &[Value]) -> Result<Rows, Error> {
        Err(unavailable())
    }

    pub fn execute(_sql: &str, _params: &[Value]) -> Result<u64, Error> {
        Err(unavailable())
    }

    pub fn begin() -> Result<(), Error> {
        Err(unavailable())
    }

    pub fn commit() -> Result<(), Error> {
        Err(unavailable())
    }

    pub fn rollback() -> Result<(), Error> {
        Err(unavailable())
    }

    fn unavailable() -> Error {
        Error::Unavailable(super::NATIVE.to_string())
    }
}

pub mod kv {
    use crate::bindings::platform::app::kv::{Entry, Error};

    pub fn get(_key: &str) -> Result<Option<Vec<u8>>, Error> {
        Err(unavailable())
    }

    pub fn set(_key: &str, _value: &[u8], _ttl_seconds: Option<u64>) -> Result<(), Error> {
        Err(unavailable())
    }

    pub fn delete(_key: &str) -> Result<(), Error> {
        Err(unavailable())
    }

    pub fn list(_prefix: &str, _start_after: Option<&str>, _limit: u32) -> Result<Vec<Entry>, Error> {
        Err(unavailable())
    }

    fn unavailable() -> Error {
        Error::Unavailable(super::NATIVE.to_string())
    }
}

pub mod secrets {
    pub fn get(_name: &str) -> Option<String> {
        None
    }
}

pub mod crypto {
    use crate::crypto::Error;

    pub fn seal(_label: &str, _plaintext: &[u8]) -> Result<String, Error> {
        Err(Error::Unavailable(super::NATIVE.to_string()))
    }

    pub fn open(_label: &str, _sealed: &str) -> Option<Vec<u8>> {
        None
    }

    pub fn sign(_label: &str, _value: &[u8], _ttl_seconds: u64) -> Result<String, Error> {
        Err(Error::Unavailable(super::NATIVE.to_string()))
    }

    pub fn verify(_label: &str, _token: &str) -> Option<Vec<u8>> {
        None
    }
}
