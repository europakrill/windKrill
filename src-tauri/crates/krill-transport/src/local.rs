//! Local shell transport.
//!
//! On Windows this wraps ConPTY (CreatePseudoConsole); on Unix a PTY via
//! `portable-pty`. M2 implements the real backends; this module defines the
//! shared spawn options and provides a loopback test double so the engine
//! pipeline is exercisable on any platform.

use crate::{Transport, TransportError};
use std::future::Future;

/// How to spawn the shell inside the PTY.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub shell: ShellKind,
    pub initial_cols: u16,
    pub initial_rows: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellKind {
    /// User default (COMSPEC / $SHELL).
    Default,
    PowerShell,
    Cmd,
    Wsl(Option<String>),
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            shell: ShellKind::Default,
            initial_cols: 120,
            initial_rows: 30,
        }
    }
}

/// In-memory loopback transport used by tests on every platform:
/// `write` echoes input back as output.
pub struct LoopbackTransport {
    pending: std::collections::VecDeque<u8>,
    cols: u16,
    rows: u16,
}

impl LoopbackTransport {
    pub fn new(opts: &SpawnOptions) -> Self {
        Self {
            pending: std::collections::VecDeque::new(),
            cols: opts.initial_cols,
            rows: opts.initial_rows,
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }
}

impl Transport for LoopbackTransport {
    fn write(&mut self, data: &[u8]) -> impl Future<Output = Result<usize, TransportError>> + Send {
        self.pending.extend(data.iter().copied());
        std::future::ready(Ok(data.len()))
    }

    fn read(
        &mut self,
        buf: &mut [u8],
    ) -> impl Future<Output = Result<usize, TransportError>> + Send {
        let n = buf.len().min(self.pending.len());
        for slot in buf.iter_mut().take(n) {
            *slot = self.pending.pop_front().unwrap_or(0);
        }
        std::future::ready(Ok(n))
    }

    fn resize(
        &mut self,
        cols: u16,
        rows: u16,
    ) -> impl Future<Output = Result<(), TransportError>> + Send {
        self.cols = cols;
        self.rows = rows;
        std::future::ready(Ok(()))
    }
}
