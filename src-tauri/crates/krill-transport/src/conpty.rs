//! Windows ConPTY adapter backed by WezTerm's `portable-pty`.
//!
//! All blocking pipe I/O and `ClosePseudoConsole` work stays off Tokio/UI
//! threads.  In particular, the output thread switches from bounded delivery
//! to drain-and-discard during shutdown; this avoids the documented pre-24H2
//! Windows deadlock where closing a pseudo console waits for output drainage.

use crate::local::{ShellKind, SpawnOptions};
use crate::{BoxFuture, Transport, TransportError};
use krill_core::validate_screen_size;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize, SlavePty};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};

const READ_CHUNK_SIZE: usize = 16 * 1024;
const READ_QUEUE_DEPTH: usize = 32;
const COMMAND_QUEUE_DEPTH: usize = 32;
const BACKPRESSURE_POLL: Duration = Duration::from_millis(2);
const OWNER_INIT_TIMEOUT: Duration = Duration::from_secs(10);
const SPAWN_TIMEOUT: Duration = Duration::from_secs(12);
const CHILD_EXIT_POLLS: usize = 200;
const CHILD_EXIT_POLL: Duration = Duration::from_millis(10);

type BackendResult<T = ()> = Result<T, String>;
type ReadResult = Result<Vec<u8>, io::Error>;

enum WriteCommand {
    Write {
        data: Vec<u8>,
        ack: oneshot::Sender<BackendResult<usize>>,
    },
}

enum ControlCommand {
    Resize {
        cols: u16,
        rows: u16,
        ack: oneshot::Sender<BackendResult>,
    },
    Close {
        ack: Option<oneshot::Sender<BackendResult>>,
    },
}

/// A local Windows shell hosted by the operating system ConPTY API.
pub struct ConPtyTransport {
    write_tx: Mutex<Option<mpsc::Sender<WriteCommand>>>,
    control_tx: Mutex<Option<mpsc::Sender<ControlCommand>>>,
    read_state: TokioMutex<ReadState>,
    shutdown: Arc<AtomicBool>,
}

struct ReadState {
    receiver: mpsc::Receiver<ReadResult>,
    pending: VecDeque<u8>,
}

impl ConPtyTransport {
    /// Spawn ConPTY without blocking the Tokio or Tauri UI thread. Both the
    /// async wait and the owner-thread initialization handshake are bounded.
    pub async fn spawn(options: &SpawnOptions) -> Result<Self, TransportError> {
        validate_size(options.initial_cols, options.initial_rows)?;
        let options = options.clone();
        let task = tokio::task::spawn_blocking(move || Self::spawn_blocking(options));
        match tokio::time::timeout(SPAWN_TIMEOUT, task).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(TransportError::Backend(format!(
                "ConPTY initialization task failed: {error}"
            ))),
            Err(_) => Err(TransportError::Backend(format!(
                "ConPTY initialization timed out after {} seconds",
                SPAWN_TIMEOUT.as_secs()
            ))),
        }
    }

    fn spawn_blocking(options: SpawnOptions) -> Result<Self, TransportError> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: options.initial_rows,
                cols: options.initial_cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(backend_error)?;
        let reader = pair.master.try_clone_reader().map_err(backend_error)?;
        let writer = pair.master.take_writer().map_err(backend_error)?;

        let (read_tx, read_rx) = mpsc::channel(READ_QUEUE_DEPTH);
        let (write_tx, write_rx) = mpsc::channel(COMMAND_QUEUE_DEPTH);
        let (control_tx, control_rx) = mpsc::channel(COMMAND_QUEUE_DEPTH);
        let shutdown = Arc::new(AtomicBool::new(false));
        let owner_shutdown = Arc::clone(&shutdown);
        let command = command_for(&options);
        let (init_tx, init_rx) = std_mpsc::sync_channel(1);

        thread::Builder::new()
            .name("krill-conpty-owner".into())
            .spawn(move || {
                owner_start(
                    pair.master,
                    pair.slave,
                    command,
                    reader,
                    writer,
                    read_tx,
                    write_rx,
                    control_rx,
                    owner_shutdown,
                    init_tx,
                );
            })
            .map_err(TransportError::Io)?;

        match init_rx.recv_timeout(OWNER_INIT_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                write_tx: Mutex::new(Some(write_tx)),
                control_tx: Mutex::new(Some(control_tx)),
                read_state: TokioMutex::new(ReadState {
                    receiver: read_rx,
                    pending: VecDeque::new(),
                }),
                shutdown,
            }),
            Ok(Err(error)) => Err(TransportError::Backend(error)),
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                shutdown.store(true, Ordering::Release);
                Err(TransportError::Backend(format!(
                    "ConPTY owner initialization timed out after {} seconds",
                    OWNER_INIT_TIMEOUT.as_secs()
                )))
            }
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(TransportError::Backend(
                "ConPTY owner exited during initialization".into(),
            )),
        }
    }
}

impl Transport for ConPtyTransport {
    fn read<'a>(&'a self, buffer: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(async move {
            if buffer.is_empty() {
                return Ok(0);
            }
            let mut state = self.read_state.lock().await;
            if state.pending.is_empty() {
                match state.receiver.recv().await {
                    Some(Ok(bytes)) => state.pending.extend(bytes),
                    Some(Err(error)) => return Err(TransportError::Io(error)),
                    None => return Ok(0),
                }
            }

            let count = buffer.len().min(state.pending.len());
            for byte in buffer.iter_mut().take(count) {
                *byte = state
                    .pending
                    .pop_front()
                    .expect("pending length was checked");
            }
            Ok(count)
        })
    }

    fn write<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(async move {
            let tx = {
                let sender = self.write_tx.lock().map_err(|_| {
                    TransportError::Backend("ConPTY writer state is poisoned".into())
                })?;
                sender.as_ref().cloned().ok_or(TransportError::Closed)?
            };
            let (ack, result) = oneshot::channel();
            tx.send(WriteCommand::Write {
                data: data.to_vec(),
                ack,
            })
            .await
            .map_err(|_| TransportError::Closed)?;
            result
                .await
                .map_err(|_| TransportError::Closed)?
                .map_err(TransportError::Backend)
        })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            validate_size(cols, rows)?;
            let tx = {
                let sender = self.control_tx.lock().map_err(|_| {
                    TransportError::Backend("ConPTY control state is poisoned".into())
                })?;
                sender.as_ref().cloned().ok_or(TransportError::Closed)?
            };
            let (ack, result) = oneshot::channel();
            tx.send(ControlCommand::Resize { cols, rows, ack })
                .await
                .map_err(|_| TransportError::Closed)?;
            result
                .await
                .map_err(|_| TransportError::Closed)?
                .map_err(TransportError::Backend)
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(async move {
            let tx = {
                let mut control = self.control_tx.lock().map_err(|_| {
                    TransportError::Backend("ConPTY control state is poisoned".into())
                })?;
                let Some(tx) = control.take() else {
                    return Ok(());
                };
                self.shutdown.store(true, Ordering::Release);
                self.write_tx
                    .lock()
                    .map_err(|_| TransportError::Backend("ConPTY writer state is poisoned".into()))?
                    .take();
                tx
            };
            let (ack, result) = oneshot::channel();
            tx.send(ControlCommand::Close { ack: Some(ack) })
                .await
                .map_err(|_| TransportError::Closed)?;
            drop(tx);
            result
                .await
                .map_err(|_| TransportError::Closed)?
                .map_err(TransportError::Backend)
        })
    }
}

impl Drop for ConPtyTransport {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        match self.write_tx.get_mut() {
            Ok(sender) => {
                sender.take();
            }
            Err(poisoned) => {
                poisoned.into_inner().take();
            }
        }
        let sender = match self.control_tx.get_mut() {
            Ok(sender) => sender.take(),
            Err(poisoned) => poisoned.into_inner().take(),
        };
        if let Some(tx) = sender {
            let _ = tx.try_send(ControlCommand::Close { ack: None });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn owner_start(
    master: Box<dyn MasterPty + Send>,
    slave: Box<dyn SlavePty + Send>,
    command: CommandBuilder,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    read_tx: mpsc::Sender<ReadResult>,
    write_rx: mpsc::Receiver<WriteCommand>,
    control_rx: mpsc::Receiver<ControlCommand>,
    shutdown: Arc<AtomicBool>,
    init_tx: std_mpsc::SyncSender<BackendResult>,
) {
    let mut child = match slave.spawn_command(command) {
        Ok(child) => child,
        Err(error) => {
            let _ = init_tx.send(Err(error.to_string()));
            return;
        }
    };
    drop(slave);

    let reader_shutdown = Arc::clone(&shutdown);
    let reader_thread = match thread::Builder::new()
        .name("krill-conpty-reader".into())
        .spawn(move || reader_loop(reader, read_tx, reader_shutdown))
    {
        Ok(thread) => thread,
        Err(error) => {
            let _ = child.kill();
            drop(master);
            let _ = init_tx.send(Err(format!("failed to start ConPTY reader: {error}")));
            return;
        }
    };

    let writer_thread = match thread::Builder::new()
        .name("krill-conpty-writer".into())
        .spawn(move || writer_loop(writer, write_rx))
    {
        Ok(thread) => thread,
        Err(error) => {
            shutdown.store(true, Ordering::Release);
            let _ = child.kill();
            drop(master);
            let _ = reader_thread.join();
            let _ = init_tx.send(Err(format!("failed to start ConPTY writer: {error}")));
            return;
        }
    };

    if init_tx.send(Ok(())).is_err() {
        shutdown.store(true, Ordering::Release);
    }
    owner_loop(
        master,
        child,
        control_rx,
        shutdown,
        reader_thread,
        writer_thread,
    );
}

fn owner_loop(
    master: Box<dyn MasterPty + Send>,
    mut child: Box<dyn Child + Send + Sync>,
    mut control_rx: mpsc::Receiver<ControlCommand>,
    shutdown: Arc<AtomicBool>,
    reader_thread: JoinHandle<()>,
    writer_thread: JoinHandle<()>,
) {
    let mut close_ack = None;
    while let Some(command) = control_rx.blocking_recv() {
        match command {
            ControlCommand::Resize { cols, rows, ack } => {
                let result = master
                    .resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|error| error.to_string());
                let _ = ack.send(result);
            }
            ControlCommand::Close { ack } => {
                close_ack = ack;
                break;
            }
        }
    }

    shutdown.store(true, Ordering::Release);
    let result = teardown(master, &mut child, reader_thread, writer_thread);
    if let Some(ack) = close_ack {
        let _ = ack.send(result);
    }
}

fn teardown(
    master: Box<dyn MasterPty + Send>,
    child: &mut Box<dyn Child + Send + Sync>,
    reader_thread: JoinHandle<()>,
    writer_thread: JoinHandle<()>,
) -> BackendResult {
    // portable-pty 0.9.0 loses the Windows TerminateProcess result.  We still
    // invoke it, then independently verify process exit below.
    let _ = child.kill();

    // On Windows <= 11 23H2 this may wait for ConPTY output to be drained.
    // The reader thread is already in drain-and-discard mode and runs in
    // parallel, so this blocking call never runs on a Tokio/UI thread.
    drop(master);

    let reader_result = reader_thread.join();
    let writer_result = writer_thread.join();
    if reader_result.is_err() {
        return Err("ConPTY reader thread panicked".into());
    }
    if writer_result.is_err() {
        return Err("ConPTY writer thread panicked".into());
    }

    for _ in 0..CHILD_EXIT_POLLS {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => thread::sleep(CHILD_EXIT_POLL),
            Err(error) => return Err(format!("failed to query ConPTY child exit: {error}")),
        }
    }
    Err("ConPTY child did not exit within two seconds".into())
}

fn reader_loop(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::Sender<ReadResult>,
    shutdown: Arc<AtomicBool>,
) {
    let mut buffer = vec![0_u8; READ_CHUNK_SIZE];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if shutdown.load(Ordering::Acquire) {
                    continue;
                }
                let mut message = Ok(buffer[..count].to_vec());
                loop {
                    if shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    match tx.try_send(message) {
                        Ok(()) => break,
                        Err(mpsc::error::TrySendError::Full(returned)) => {
                            message = returned;
                            thread::sleep(BACKPRESSURE_POLL);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            shutdown.store(true, Ordering::Release);
                            break;
                        }
                    }
                }
            }
            Err(mut error) => {
                // A full read queue must not swallow real I/O errors as EOF:
                // keep retrying (bounded by shutdown) so the consumer observes
                // the error instead of a clean channel close.
                loop {
                    if shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    match tx.try_send(Err(error)) {
                        Ok(()) => break,
                        Err(mpsc::error::TrySendError::Full(returned)) => match returned {
                            ReadResult::Err(retry) => {
                                error = retry;
                                thread::sleep(BACKPRESSURE_POLL);
                            }
                            _ => unreachable!("only Io errors are enqueued here"),
                        },
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                break;
            }
        }
    }
}

fn writer_loop(mut writer: Box<dyn Write + Send>, mut rx: mpsc::Receiver<WriteCommand>) {
    while let Some(command) = rx.blocking_recv() {
        match command {
            WriteCommand::Write { data, ack } => {
                let result = writer
                    .write_all(&data)
                    .and_then(|()| writer.flush())
                    .map(|()| data.len())
                    .map_err(|error| error.to_string());
                let failed = result.is_err();
                let _ = ack.send(result);
                if failed {
                    break;
                }
            }
        }
    }
}

fn command_for(options: &SpawnOptions) -> CommandBuilder {
    let mut command = match &options.shell {
        ShellKind::Default => CommandBuilder::new_default_prog(),
        ShellKind::PowerShell => {
            let mut command = CommandBuilder::new("powershell.exe");
            command.arg("-NoLogo");
            command
        }
        ShellKind::Cmd => {
            let mut command = CommandBuilder::new("cmd.exe");
            command.arg("/Q");
            command
        }
        ShellKind::Wsl(distro) => {
            let mut command = CommandBuilder::new("wsl.exe");
            if let Some(distro) = distro {
                command.arg("-d");
                command.arg(distro);
            }
            command
        }
    };
    command.env("TERM", "xterm-256color");
    command
}

fn validate_size(cols: u16, rows: u16) -> Result<(), TransportError> {
    // Share the screen model's hard limits so the PTY and Screen can never
    // disagree about what a legal size is (and can never over-allocate).
    validate_screen_size(cols, rows).map_err(|error| TransportError::Backend(error.to_string()))
}

fn backend_error(error: impl std::fmt::Display) -> TransportError {
    TransportError::Backend(error.to_string())
}
