use vte::{AnsiHandler, Perform};

/// Configuration for the VT parser frontend.
#[derive(Debug, Clone)]
pub struct ParserConfig {
    /// Report mouse events (xterm mouse protocol 1000/1002/1006).
    pub mouse_protocol: bool,
    /// Enable bracketed paste mode handling (2004).
    pub bracketed_paste: bool,
}

impl Default for ParserConfig {
    fn default() -> Self {
        Self { mouse_protocol: true, bracketed_paste: true }
    }
}

/// Thin facade over `vte::Parser`. The screen-model side implements
/// `Perform` in krill-core; this type exists so krill-core does not
/// need to know which parsing engine is underneath.
#[derive(Debug)]
pub struct VtParser {
    inner: vte::Parser,
    #[allow(dead_code)]
    config: ParserConfig,
}

impl VtParser {
    pub fn new(config: ParserConfig) -> Self {
        Self { inner: vte::Parser::new(), config }
    }

    /// Feed raw PTY bytes into the parser; events are dispatched to `handler`.
    pub fn advance<H: AnsiHandler + Perform>(&mut self, bytes: &[u8], handler: &mut H) {
        // vte 0.13 API: advance(&mut performer, bytes)
        self.inner.advance(handler, bytes);
    }
}
