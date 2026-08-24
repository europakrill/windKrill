//! Session pump: owns one Transport + parser + Screen and moves bytes
//! from the wire into the terminal model. One `Session` per tab/pane.

use krill_core::Screen;
use krill_transport::{Transport, TransportError};
use krill_vt::{ParserConfig, TermPerformer, VtParser};

pub mod actor;
pub use actor::{spawn_session, SessionHandle, SessionStatus};

/// Read chunk size for one pump iteration.
const READ_BUF: usize = 16 * 1024;

pub struct Session<T: Transport> {
    transport: T,
    parser: VtParser,
    performer: TermPerformer,
    buf: Vec<u8>,
}

impl<T: Transport> Session<T> {
    pub fn new(transport: T, screen: Screen) -> Self {
        Self {
            transport,
            parser: VtParser::new(ParserConfig::default()),
            performer: TermPerformer::new(screen),
            buf: vec![0u8; READ_BUF],
        }
    }

    /// Read once from the transport and push parsed events into the screen.
    /// Returns the number of raw bytes consumed (0 = no data right now).
    pub async fn pump(&mut self) -> Result<usize, TransportError> {
        let n = self.transport.read(&mut self.buf).await?;
        if n > 0 {
            self.parser.advance(&self.buf[..n], &mut self.performer);
            self.performer.flush();
            for response in self.performer.take_responses() {
                let written = self.transport.write(&response).await?;
                if written != response.len() {
                    return Err(TransportError::Backend(format!(
                        "short terminal response write: {written}/{} bytes",
                        response.len()
                    )));
                }
            }
        }
        Ok(n)
    }

    /// Send user input to the session.
    pub async fn send_input(&mut self, data: &[u8]) -> Result<usize, TransportError> {
        self.transport.write(data).await
    }

    /// Notify the PTY of a viewport resize.
    pub async fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TransportError> {
        krill_core::validate_screen_size(cols, rows)
            .map_err(|error| TransportError::Backend(error.to_string()))?;
        self.transport.resize(cols, rows).await?;
        // Transport already validated the size; resize cannot fail here.
        let _ = self.performer.screen.resize(cols, rows);
        Ok(())
    }

    pub fn screen(&self) -> &Screen {
        &self.performer.screen
    }

    pub fn into_parts(self) -> (T, Screen) {
        let mut performer = self.performer;
        performer.flush();
        (self.transport, performer.screen)
    }
}
