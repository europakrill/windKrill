//! Local shell transport. Windows/ConPTY wiring is M2 scope; on other
//! platforms this module compiles but returns `Unsupported` for now.

use crate::{Transport, TransportError};

pub struct LocalShell;

impl Transport for LocalShell {
    fn write(&mut self, _data: &[u8]) -> Result<usize, TransportError> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "ConPTY backend not wired yet").into())
    }
}
