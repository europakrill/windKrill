//! Session transport layer.
//!
//! M2 scope: async byte-stream abstraction over ConPTY (Windows) and a
//! portable fallback. SSH/Telnet/Serial implement the same trait later.

use std::future::Future;
use std::pin::Pin;
use thiserror::Error;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub mod local;

#[cfg(windows)]
mod conpty;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("session already closed")]
    Closed,
}

/// A bidirectional byte transport to a session (local PTY, SSH channel, ...).
///
/// `BoxFuture` keeps this trait object-safe so local, SSH, Telnet and serial
/// transports can be selected at runtime.
pub trait Transport: Send + Sync {
    /// Write all user input to the session, preserving ordering.
    fn write<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>>;
    /// Read raw output bytes. `0` means permanent EOF; no-data stays pending.
    fn read<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>>;
    /// Resize the pseudo terminal (no-op for socket-based transports).
    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), TransportError>>;
    /// Close the transport and wait for its blocking backend teardown.
    fn close(&self) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}
