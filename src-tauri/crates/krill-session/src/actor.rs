//! Concurrent session actor.
//!
//! A single Tokio task owns the VT parser and screen while the transport is
//! shared behind an `Arc`, so reads and writes progress concurrently: a slow
//! or blocked write can no longer starve output reads (and through ConPTY
//! backpressure, the child process itself).
//!
//! Liveness: writes are kept in a pending queue, never awaited inline. Close
//! (or dropping every handle) abandons any in-flight write instead of waiting
//! for it, so even a transport whose writer thread is wedged can always be
//! closed — the actor task exits and releases the `Arc`, letting the
//! transport's own Drop cleanup run. EOF while a write is in flight also
//! abandons the queue rather than spinning on `read -> Ok(0)`.

use krill_core::Screen;
use krill_transport::{Transport, TransportError};
use krill_vt::{ParserConfig, TermPerformer, VtParser};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, watch};

const COMMAND_CAPACITY: usize = 64;
const READ_BUFFER_SIZE: usize = 64 * 1024;
/// Hard caps for the internal write queue. Both must hold: entry count bounds
/// ack-channel pressure, byte count bounds memory even with huge pastes.
const WRITE_QUEUE_MAX_ENTRIES: usize = 64;
const WRITE_QUEUE_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Upper bound on waiting for backend teardown; the actor always exits after
/// this so dropping every handle can never wedge the task forever.
const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Hard caps for the protocol-response queue (CPR replies etc.). A runaway
/// child flooding DSR queries must not grow memory without bound; overflow
/// drops the newest replies rather than blocking output processing.
const RESPONSE_QUEUE_MAX_ENTRIES: usize = 64;
const RESPONSE_QUEUE_MAX_BYTES: usize = 256 * 1024;

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

enum Wake {
    Command(Option<SessionCommand>),
    Read(Result<usize, TransportError>),
    Written(Result<usize, TransportError>),
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

/// A write accepted from the wire but not yet handed to the transport.
struct QueuedWrite {
    data: Vec<u8>,
    ack: Option<oneshot::Sender<Result<usize, TransportError>>>,
}

impl QueuedWrite {
    fn bytes(&self) -> usize {
        self.data.len()
    }
}

/// The one queue item currently being polled against reads and commands.
/// The future owns the bytes; we only keep the length for the ack.
struct PendingWrite {
    len: usize,
    ack: Option<oneshot::Sender<Result<usize, TransportError>>>,
    future: Pin<Box<dyn Future<Output = Result<usize, TransportError>> + Send>>,
}

fn start_write(transport: &Arc<dyn Transport>, queued: QueuedWrite) -> PendingWrite {
    // The future takes full ownership of the bytes — no duplication while the
    // write is in flight; the ack reports the length via `len`.
    let owned = queued.data;
    let len = owned.len();
    let transport = Arc::clone(transport);
    PendingWrite {
        future: Box::pin(async move {
            transport.write(&owned).await?;
            Ok(len)
        }),
        len,
        ack: queued.ack,
    }
}

fn abandon_writes(pending: &mut Option<PendingWrite>, queued: &mut VecDeque<QueuedWrite>) {
    if let Some(mut write) = pending.take() {
        if let Some(ack) = write.ack.take() {
            let _ = ack.send(Err(TransportError::Closed));
        }
    }
    for mut write in queued.drain(..) {
        if let Some(ack) = write.ack.take() {
            let _ = ack.send(Err(TransportError::Closed));
        }
    }
}

/// Abandon everything (pending write, user input queue, protocol replies).
macro_rules! abandon_all {
    ($pending:expr, $write_queue:expr, $response_queue:expr) => {
        abandon_writes(&mut $pending, &mut $write_queue);
        abandon_writes(&mut None, &mut $response_queue);
    };
}

/// Total bytes currently queued (not counting the in-flight write).
fn queued_bytes(queued: &VecDeque<QueuedWrite>) -> usize {
    queued.iter().map(QueuedWrite::bytes).sum()
}

/// Close the transport with a hard bound so a backend teardown that never
/// finishes cannot hang the actor (and with it, transport release) forever.
/// The bounded close is best-effort: on timeout we still publish status and
/// exit; the ConPTY layer's own Drop remains the last-resort cleanup.
async fn bounded_close(transport: &Arc<dyn Transport>) -> Result<(), TransportError> {
    match tokio::time::timeout(CLOSE_TIMEOUT, transport.close()).await {
        Ok(result) => result,
        Err(_) => Err(TransportError::Backend(format!(
            "transport close timed out after {CLOSE_TIMEOUT:?}"
        ))),
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
    // At most one write is in flight at a time. User input remains ordered;
    // protocol replies may move ahead of queued (not-yet-written) input.
    let mut pending: Option<PendingWrite> = None;
    let mut write_queue: VecDeque<QueuedWrite> = VecDeque::new();
    let mut response_queue: VecDeque<QueuedWrite> = VecDeque::new();
    let mut response_bytes: usize = 0;

    loop {
        if pending.is_none() {
            // Protocol replies take priority over not-yet-written user input,
            // but each queue keeps its own FIFO order (no cross-batch reversal).
            if let Some(queued) = response_queue.pop_front() {
                response_bytes -= queued.bytes();
                pending = Some(start_write(&transport, queued));
            } else if let Some(queued) = write_queue.pop_front() {
                pending = Some(start_write(&transport, queued));
            }
        }
        // Start a fresh read each iteration; the previous one was dropped when
        // the select ended, releasing its `&mut buf` borrow. Transports must
        // keep `read` cancellation-safe.
        let wake = if let Some(write) = pending.as_mut() {
            // Poll the in-flight write as a real select branch alongside read
            // and control commands. Close/None therefore always preempt it,
            // while a completed write resolves its ack without a busy loop.
            tokio::select! {
                command = commands.recv() => Wake::Command(command),
                read = transport.read(&mut buf) => Wake::Read(read),
                written = write.future.as_mut() => Wake::Written(written),
            }
        } else {
            tokio::select! {
                command = commands.recv() => Wake::Command(command),
                read = transport.read(&mut buf) => Wake::Read(read),
            }
        };

        match wake {
            Wake::Command(Some(SessionCommand::Input { data, ack })) => {
                // Admission control: bounded in entries AND total bytes so a
                // wedged write can never grow the queue without limit.
                let would_exceed = write_queue.len() >= WRITE_QUEUE_MAX_ENTRIES
                    || queued_bytes(&write_queue) + data.len() > WRITE_QUEUE_MAX_BYTES;
                if would_exceed {
                    let _ = ack.send(Err(TransportError::Backend(
                        "session write queue is full".into(),
                    )));
                } else {
                    write_queue.push_back(QueuedWrite {
                        data,
                        ack: Some(ack),
                    });
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
                // Abandon any in-flight write: liveness beats delivery when
                // the caller has asked to tear the session down.
                abandon_all!(pending, write_queue, response_queue);
                let result = bounded_close(&transport).await;
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
                // Every handle dropped: same teardown path as explicit close,
                // so a wedged write cannot keep the task (and transport Arc)
                // alive forever.
                abandon_all!(pending, write_queue, response_queue);
                finish(&transport, &status_tx, SessionStatus::Closed).await;
                return;
            }
            Wake::Read(Ok(0)) => {
                // EOF: abandon any queued write too, otherwise a wedged write
                // combined with instant EOF would busy-spin this loop forever.
                abandon_all!(pending, write_queue, response_queue);
                // EOF is only "clean" when teardown also succeeds; otherwise
                // observers must see the failure instead of a normal Eof.
                let status = match bounded_close(&transport).await {
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
                    // Bounded FIFO: protocol replies keep generation order and
                    // take priority over queued user input, but a runaway child
                    // flooding DSR queries cannot grow memory without limit —
                    // overflow drops the newest replies.
                    if response_queue.len() >= RESPONSE_QUEUE_MAX_ENTRIES
                        || response_bytes + response.len() > RESPONSE_QUEUE_MAX_BYTES
                    {
                        continue;
                    }
                    response_bytes += response.len();
                    response_queue.push_back(QueuedWrite {
                        data: response,
                        ack: None,
                    });
                }
                screen_tx.send_replace(performer.screen.clone());
            }
            Wake::Written(Ok(_)) => {
                if let Some(mut write) = pending.take() {
                    if let Some(ack) = write.ack.take() {
                        let _ = ack.send(Ok(write.len));
                    }
                }
            }
            Wake::Written(Err(error)) => {
                // Preserve the error variant instead of flattening to Backend.
                let display = error.to_string();
                if let Some(mut write) = pending.take() {
                    if let Some(ack) = write.ack.take() {
                        let _ = ack.send(Err(error));
                    }
                }
                abandon_all!(pending, write_queue, response_queue);
                fail_and_close(&transport, &status_tx, display).await;
                return;
            }
            Wake::Read(Err(error)) => {
                abandon_all!(pending, write_queue, response_queue);
                fail_and_close(&transport, &status_tx, error.to_string()).await;
                return;
            }
        }
    }
}

async fn finish(
    transport: &Arc<dyn Transport>,
    status_tx: &watch::Sender<SessionStatus>,
    success: SessionStatus,
) {
    let status = match bounded_close(transport).await {
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
    // Publish the failure BEFORE teardown so a caller observing the error ack
    // or status never still sees Running, even if teardown is slow.
    status_tx.send_replace(SessionStatus::Failed(error));
    let _ = bounded_close(transport).await;
}
