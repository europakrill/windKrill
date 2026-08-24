//! Concurrent session actor.
//!
//! A single Tokio task owns the VT parser and screen while the transport is
//! shared behind an `Arc`, so reads and writes progress concurrently: a slow
//! or blocked write can no longer starve output reads (and through ConPTY
//! backpressure, the child process itself). In-flight writes are kept in the
//! actor's select state instead of being awaited inline.

use krill_core::Screen;
use krill_transport::{Transport, TransportError};
use krill_vt::{ParserConfig, TermPerformer, VtParser};
use std::future::Future;
use std::pin::{pin, Pin};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

const COMMAND_CAPACITY: usize = 64;
const READ_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Eof,
    Closed,
    Failed(String),
}

#[derive(Clone)]
pub struct SessionHandle {
    commands: mpsc::Sender<SessionCommand>,
    screen: watch::Receiver<Screen>,
    status: watch::Receiver<SessionStatus>,
}

impl SessionHandle {
    pub async fn send_input(&self, data: &[u8]) -> Result<usize, TransportError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Input {
                data: data.to_vec(),
                ack: ack_tx,
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        ack_rx.await.map_err(|_| TransportError::Closed)?
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> Result<(), TransportError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Resize {
                cols,
                rows,
                ack: ack_tx,
            })
            .await
            .map_err(|_| TransportError::Closed)?;
        ack_rx.await.map_err(|_| TransportError::Closed)?
    }

    pub async fn close(&self) -> Result<(), TransportError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Close { ack: ack_tx })
            .await
            .map_err(|_| TransportError::Closed)?;
        ack_rx.await.map_err(|_| TransportError::Closed)?
    }

    pub fn screen(&self) -> Screen {
        self.screen.borrow().clone()
    }

    pub fn status(&self) -> SessionStatus {
        self.status.borrow().clone()
    }

    pub async fn screen_changed(&mut self) -> Result<(), TransportError> {
        self.screen
            .changed()
            .await
            .map_err(|_| TransportError::Closed)
    }

    pub async fn status_changed(&mut self) -> Result<(), TransportError> {
        self.status
            .changed()
            .await
            .map_err(|_| TransportError::Closed)
    }
}

enum SessionCommand {
    Input {
        data: Vec<u8>,
        ack: oneshot::Sender<Result<usize, TransportError>>,
    },
    Resize {
        cols: u16,
        rows: u16,
        ack: oneshot::Sender<Result<(), TransportError>>,
    },
    Close {
        ack: oneshot::Sender<Result<(), TransportError>>,
    },
}

/// One unit of work drained from the command queue between loop iterations.
enum Wake {
    Command(Option<SessionCommand>),
    Read(Result<usize, TransportError>),
}

pub fn spawn_session<T>(transport: T, screen: Screen) -> SessionHandle
where
    T: Transport + 'static,
{
    let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
    let (screen_tx, screen_rx) = watch::channel(screen.clone());
    let (status_tx, status_rx) = watch::channel(SessionStatus::Running);

    tokio::spawn(run_session(
        Arc::new(transport),
        screen,
        command_rx,
        screen_tx,
        status_tx,
    ));

    SessionHandle {
        commands: command_tx,
        screen: screen_rx,
        status: status_rx,
    }
}

async fn run_session(
    transport: Arc<dyn Transport>,
    screen: Screen,
    mut commands: mpsc::Receiver<SessionCommand>,
    screen_tx: watch::Sender<Screen>,
    status_tx: watch::Sender<SessionStatus>,
) {
    let mut parser = VtParser::new(ParserConfig::default());
    let mut performer = TermPerformer::new(screen);
    // The buffer must outlive the read future across select iterations; the
    // read future itself borrows it for exactly one poll cycle.
    let mut buf = vec![0_u8; READ_BUFFER_SIZE];

    loop {
        // Start a fresh read each iteration; the previous one was dropped when
        // the select ended, releasing its `&mut buf` borrow. Transports must
        // keep `read` cancellation-safe.
        let wake = tokio::select! {
            biased;
            command = commands.recv() => Wake::Command(command),
            read = transport.read(&mut buf) => Wake::Read(read),
        };

        match wake {
            Wake::Command(Some(SessionCommand::Input { data, ack })) => {
                // Drive the write concurrently with reads: poll it alongside
                // the next select iterations until it completes.
                let write = transport.write(&data);
                let mut write = pin!(write);
                let result = drive_write_concurrent(
                    &mut write,
                    &mut commands,
                    &transport,
                    &mut buf,
                    &mut parser,
                    &mut performer,
                    &screen_tx,
                )
                .await;
                match result {
                    Ok(()) => {
                        let _ = ack.send(Ok(data.len()));
                    }
                    Err(error) => {
                        fail_and_close(&transport, &status_tx, error.to_string()).await;
                        let _ = ack.send(Err(TransportError::Closed));
                        return;
                    }
                }
            }
            Wake::Command(Some(SessionCommand::Resize { cols, rows, ack })) => {
                // Validate against the shared screen limits before touching
                // the PTY so both sides stay consistent or both refuse.
                let result = match krill_core::validate_screen_size(cols, rows) {
                    Ok(()) => transport.resize(cols, rows).await,
                    Err(error) => Err(TransportError::Backend(error.to_string())),
                };
                match &result {
                    Ok(()) => {
                        // Transport already validated; resize cannot fail here.
                        let _ = performer.screen.resize(cols, rows);
                        screen_tx.send_replace(performer.screen.clone());
                    }
                    Err(_) => {
                        // Invalid resize is a caller error, not a session
                        // failure: report it and keep the session alive.
                    }
                }
                let _ = ack.send(result);
            }
            Wake::Command(Some(SessionCommand::Close { ack })) => {
                let result = transport.close().await;
                match &result {
                    Ok(()) => {
                        status_tx.send_replace(SessionStatus::Closed);
                    }
                    Err(error) => {
                        status_tx.send_replace(SessionStatus::Failed(error.to_string()));
                    }
                }
                let _ = ack.send(result);
                return;
            }
            Wake::Command(None) => {
                finish(&transport, &status_tx, SessionStatus::Closed).await;
                return;
            }
            Wake::Read(Ok(0)) => {
                // EOF is only "clean" when teardown also succeeds; otherwise
                // observers must see the failure instead of a normal Eof.
                let status = match transport.close().await {
                    Ok(()) => SessionStatus::Eof,
                    Err(error) => SessionStatus::Failed(error.to_string()),
                };
                status_tx.send_replace(status);
                return;
            }
            Wake::Read(Ok(n)) => {
                parser.advance(&buf[..n], &mut performer);
                performer.flush();
                for response in performer.take_responses() {
                    // Protocol responses are small and rare; still written
                    // concurrently with reads to preserve the invariant that a
                    // pending write never blocks the read side.
                    let write = transport.write(&response);
                    let mut write = pin!(write);
                    if let Err(error) = drive_write_concurrent(
                        &mut write,
                        &mut commands,
                        &transport,
                        &mut buf,
                        &mut parser,
                        &mut performer,
                        &screen_tx,
                    )
                    .await
                    {
                        fail_and_close(&transport, &status_tx, error.to_string()).await;
                        return;
                    }
                }
                screen_tx.send_replace(performer.screen.clone());
            }
            Wake::Read(Err(error)) => {
                fail_and_close(&transport, &status_tx, error.to_string()).await;
                return;
            }
        }
    }
}

/// Poll a single write future while concurrently servicing reads and inbound
/// commands. Returns when the write completes; any read/command error aborts
/// with that error so callers run their failure paths.
#[allow(clippy::too_many_arguments)]
async fn drive_write_concurrent(
    write: &mut Pin<Box<dyn Future<Output = Result<usize, TransportError>> + Send + '_>>,
    commands: &mut mpsc::Receiver<SessionCommand>,
    transport: &Arc<dyn Transport>,
    buf: &mut [u8],
    parser: &mut VtParser,
    performer: &mut TermPerformer,
    screen_tx: &watch::Sender<Screen>,
) -> Result<(), TransportError> {
    loop {
        tokio::select! {
            biased;
            command = commands.recv() => {
                match command {
                    // Queue-ordered writes: refuse new inputs while one is in
                    // flight rather than racing them on the wire.
                    Some(SessionCommand::Input { ack, .. }) => {
                        let _ = ack.send(Err(TransportError::Backend(
                            "input while a write is in flight".into(),
                        )));
                    }
                    Some(SessionCommand::Resize { cols, rows, ack }) => {
                        let result = match krill_core::validate_screen_size(cols, rows) {
                            Ok(()) => transport.resize(cols, rows).await,
                            Err(error) => Err(TransportError::Backend(error.to_string())),
                        };
                        if let Ok(()) = result {
                            let _ = performer.screen.resize(cols, rows);
                            screen_tx.send_replace(performer.screen.clone());
                        }
                        let _ = ack.send(result);
                    }
                    Some(SessionCommand::Close { ack }) => {
                        let _ = ack.send(Err(TransportError::Backend(
                            "close requested while a write is in flight".into(),
                        )));
                    }
                    None => {}
                }
            }
            read = transport.read(buf) => {
                match read? {
                    0 => {}
                    n => {
                        parser.advance(&buf[..n], performer);
                        performer.flush();
                        for response in performer.take_responses() {
                            // Nested protocol responses during an input write:
                            // write them inline (small, ordered).
                            transport.write(&response).await?;
                        }
                        screen_tx.send_replace(performer.screen.clone());
                    }
                }
            }
            written = write.as_mut() => {
                written?;
                return Ok(());
            }
        }
    }
}

async fn finish(
    transport: &Arc<dyn Transport>,
    status_tx: &watch::Sender<SessionStatus>,
    success: SessionStatus,
) {
    let status = match transport.close().await {
        Ok(()) => success,
        Err(error) => SessionStatus::Failed(error.to_string()),
    };
    status_tx.send_replace(status);
}

async fn fail_and_close(
    transport: &Arc<dyn Transport>,
    status_tx: &watch::Sender<SessionStatus>,
    error: String,
) {
    status_tx.send_replace(SessionStatus::Failed(error));
    let _ = transport.close().await;
}
