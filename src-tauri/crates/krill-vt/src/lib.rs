//! krill-vt: VT/xterm parser frontend.
//!
//! Wraps the `vte` crate behind a stable API and provides `TermPerformer`,
//! the `vte::Perform` implementation that translates parser events into
//! krill-core `Screen` mutations.

pub mod parser;
pub mod performer;

pub use parser::{ParserConfig, VtParser};
pub use performer::TermPerformer;
