//! krill-vt: VT/xterm parser frontend.
//!
//! M0 scope: wrap the `vte` crate behind a stable API that feeds
//! parsed cell/attribute events into krill-core's screen model.
//! Acceptance: vttest-style golden replay tests pass (M1).

pub mod parser;

pub use parser::{ParserConfig, VtParser};
