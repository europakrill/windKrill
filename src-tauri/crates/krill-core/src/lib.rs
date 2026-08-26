//! krill-core: the terminal screen model.
//!
//! This crate must stay GUI-free and I/O-free.

pub mod screen;
pub mod snapshot;

pub use screen::{
    validate_screen_size, Attrs, Cell, Color, Modes, Screen, ScreenSizeError, MAX_SCREEN_CELLS,
    MAX_SCREEN_DIMENSION,
};
pub use snapshot::{AttrDto, RowDto, RunDto, SnapshotDto};
