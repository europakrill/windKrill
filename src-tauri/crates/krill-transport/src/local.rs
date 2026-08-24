//! Local shell transport.
//!
//! On Windows this wraps ConPTY (CreatePseudoConsole); on Unix a PTY via
//! `portable-pty`. M2 implements the real backends; this module defines the
//! shared spawn options and provides a loopback test double so the engine
//! pipeline is exercisable on any platform.

use crate::{BoxFuture, Transport, TransportError};
use krill_core::validate_screen_size;
use std::sync::Mutex;

#[cfg(windows)]
pub use crate::conpty::ConPtyTransport;

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
    state: Mutex<LoopbackState>,
}

struct LoopbackState {
    pending: std::collections::VecDeque<u8>,
    cols: u16,
    rows: u16,
}

impl LoopbackTransport {
    pub fn new(opts: &SpawnOptions) -> Self {
        validate_screen_size(opts.initial_cols, opts.initial_rows)
            .expect("loopback dimensions must be valid");
        Self {
            state: Mutex::new(LoopbackState {
                pending: std::collections::VecDeque::new(),
                cols: opts.initial_cols,
                rows: opts.initial_rows,
            }),
        }
    }

    pub fn size(&self) -> (u16, u16) {
        let state = self.state.lock().expect("loopback state poisoned");
        (state.cols, state.rows)
    }
}

impl Transport for LoopbackTransport {
    fn write<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        let mut state = self.state.lock().expect("loopback state poisoned");
        state.pending.extend(data.iter().copied());
        Box::pin(std::future::ready(Ok(data.len())))
    }

    fn read<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        let mut state = self.state.lock().expect("loopback state poisoned");
        let n = buf.len().min(state.pending.len());
        for slot in buf.iter_mut().take(n) {
            *slot = state.pending.pop_front().unwrap_or(0);
        }
        Box::pin(std::future::ready(Ok(n)))
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), TransportError>> {
        let result = validate_screen_size(cols, rows)
            .map_err(|error| TransportError::Backend(error.to_string()))
            .map(|()| {
                let mut state = self.state.lock().expect("loopback state poisoned");
                state.cols = cols;
                state.rows = rows;
            });
        Box::pin(std::future::ready(result))
    }
}
