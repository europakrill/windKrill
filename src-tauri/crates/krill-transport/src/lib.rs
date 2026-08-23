//! krill-transport: session transport layer.
//!
//! M0 scope: trait definitions only. ConPTY implementation lands with
//! the first Windows milestone build.

use thiserror::Error;

pub mod local;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("session already closed")]
    Closed,
}

/// A bidirectional byte transport to a session (local PTY, SSH channel, ...).
pub trait Transport {
    /// Write user input to the session.
    fn write(&mut self, data: &[u8]) -> Result<usize, TransportError>;
    // M0+: async read side lands with the ConPTY implementation.
}
