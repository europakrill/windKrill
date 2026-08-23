//! Session transport layer.
//!
//! M2 scope: async byte-stream abstraction over ConPTY (Windows) and a
//! portable fallback. SSH/Telnet/Serial implement the same trait later.

use std::future::Future;
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
///
/// The read side is exposed as an `AsyncRead`-like poll so the terminal
/// engine can be driven from one tokio task per session.
pub trait Transport: Send {
    /// Write user input to the session.
    fn write(&mut self, data: &[u8]) -> impl Future<Output = Result<usize, TransportError>> + Send;
    /// Read raw output bytes from the session.
    fn read(
        &mut self,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<usize, TransportError>> + Send;
    /// Resize the pseudo terminal (no-op for socket-based transports).
    fn resize(
        &mut self,
        cols: u16,
        rows: u16,
    ) -> impl Future<Output = Result<(), TransportError>> + Send;
}
