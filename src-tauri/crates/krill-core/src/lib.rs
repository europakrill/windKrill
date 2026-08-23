//! krill-core: terminal screen buffer core.
//!
//! Owns the grid model, scrollback, rendition and DEC modes. Receives
//! parsed semantics via the `TermPerformer` bridge (krill-vt) which
//! implements `vte::Perform` against [`Screen`].
//! This crate must stay GUI-free and I/O-free.

pub mod screen;

pub use screen::{Attrs, Cell, Color, Modes, Screen};
