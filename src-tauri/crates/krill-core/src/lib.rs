//! krill-core: terminal screen buffer core.
//!
//! Owns the grid model, logical command blocks, folding/outlining,
//! timestamps and compressed scrollback. Receives VT events from
//! krill-vt. This crate must stay GUI-free and I/O-free.

pub mod screen;

pub use screen::{Cell, Screen};
