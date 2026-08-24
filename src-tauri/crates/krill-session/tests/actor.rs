use krill_core::Screen;
use krill_session::{spawn_session, SessionStatus};
use krill_transport::{BoxFuture, Transport, TransportError};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

struct PendingReadTransport {
    incoming: Arc<tokio::sync::Mutex<mpsc::Receiver<Vec<u8>>>>,
    pending: Mutex<VecDeque<u8>>,
    writes: mpsc::UnboundedSender<Vec<u8>>,
    resizes: mpsc::UnboundedSender<(u16, u16)>,
    closed: Arc<std::sync::atomic::AtomicBool>,
}

impl Transport for PendingReadTransport {
    fn write<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(async move {
            self.writes
                .send(data.to_vec())
                .map_err(|_| TransportError::Closed)?;
            Ok(data.len())
        })
    }

    fn read<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(async move {
            // Scope the std MutexGuards so none is held across the await.
            let needs_data = {
                let pending = self.pending.lock().unwrap();
                pending.is_empty()
            };
            if needs_data {
                // Async mutex: holding its guard across `.await` is fine and
                // keeps the future Send; tokio channel recv is cancel-safe, so
                // a dropped poll never loses data.
                let received = self.incoming.lock().await.recv().await;
                match received {
                    Some(data) => {
                        self.pending.lock().unwrap().extend(data);
                    }
                    None => return Ok(0),
                }
            }
            let mut pending = self.pending.lock().unwrap();
            let n = buf.len().min(pending.len());
            for slot in buf.iter_mut().take(n) {
                *slot = pending.pop_front().expect("length checked");
            }
            Ok(n)
        })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, Result<(), TransportError>> {
        let result = self
            .resizes
            .send((cols, rows))
            .map_err(|_| TransportError::Closed);
        Box::pin(std::future::ready(result))
    }

    fn close(&self) -> BoxFuture<'_, Result<(), TransportError>> {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        Box::pin(std::future::ready(Ok(())))
    }
}

#[tokio::test]
async fn actor_handles_commands_while_read_is_pending_and_drives_vt() {
    let (incoming_tx, incoming_rx) = mpsc::channel(4);
    let (writes_tx, mut writes_rx) = mpsc::unbounded_channel();
    let (resizes_tx, mut resizes_rx) = mpsc::unbounded_channel();
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let transport = PendingReadTransport {
        incoming: Arc::new(tokio::sync::Mutex::new(incoming_rx)),
        pending: Mutex::new(VecDeque::new()),
        writes: writes_tx,
        resizes: resizes_tx,
        closed: Arc::clone(&closed),
    };
    let mut handle = spawn_session(transport, Screen::new(80, 24));

    tokio::time::timeout(Duration::from_secs(1), handle.send_input(b"ping"))
        .await
        .expect("input blocked behind pending read")
        .unwrap();
    assert_eq!(writes_rx.recv().await.unwrap(), b"ping");

    tokio::time::timeout(Duration::from_secs(1), handle.resize(100, 40))
        .await
        .expect("resize blocked behind pending read")
        .unwrap();
    assert_eq!(resizes_rx.recv().await.unwrap(), (100, 40));
    handle.screen_changed().await.unwrap();
    assert_eq!((handle.screen().cols(), handle.screen().rows()), (100, 40));

    incoming_tx
        .send(b"\x1b[3;5H\x1b[6nready".to_vec())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), handle.screen_changed())
        .await
        .expect("screen update timeout")
        .unwrap();
    assert!(handle.screen().text().contains("ready"));
    assert_eq!(writes_rx.recv().await.unwrap(), b"\x1b[3;5R");

    tokio::time::timeout(Duration::from_secs(1), handle.close())
        .await
        .expect("close timeout")
        .unwrap();
    assert_eq!(handle.status(), SessionStatus::Closed);
    assert!(closed.load(std::sync::atomic::Ordering::Acquire));
}

/// Regression for the review finding "biased select + serial write starves the
/// read branch": while a write is in flight, the transport's read side must
/// still be polled concurrently.
struct ReadActiveFlag(Arc<std::sync::atomic::AtomicBool>);

impl ReadActiveFlag {
    fn load(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }

    fn store(&self, value: bool) {
        self.0.store(value, std::sync::atomic::Ordering::Release);
    }
}

struct ConcurrentReadWriteTransport {
    read_active: ReadActiveFlag,
}

impl Transport for ConcurrentReadWriteTransport {
    fn write<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(async move {
            // The old actor served Input inline in its biased select loop, so a
            // write could only complete after the read future was dropped and
            // never re-polled; this wait would then time out.
            tokio::time::timeout(Duration::from_millis(250), async {
                while !self.read_active.load() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .map_err(|_| TransportError::Backend("read was not active during write".into()))?;
            Ok(data.len())
        })
    }

    fn read<'a>(&'a self, _buf: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(async move {
            // Mark active while this future is alive. The actor must keep it
            // alive (polling) even when a write is being awaited elsewhere.
            struct ClearOnDrop(ReadActiveFlag);
            impl Drop for ClearOnDrop {
                fn drop(&mut self) {
                    self.0.store(false);
                }
            }
            let _guard = ClearOnDrop(ReadActiveFlag(Arc::clone(&self.read_active.0)));
            self.read_active.store(true);
            std::future::pending::<()>().await;
            unreachable!()
        })
    }

    fn resize(&self, _cols: u16, _rows: u16) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actor_keeps_read_active_while_a_write_is_in_flight() {
    let flag = ReadActiveFlag(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let handle = spawn_session(
        ConcurrentReadWriteTransport {
            read_active: ReadActiveFlag(Arc::clone(&flag.0)),
        },
        Screen::new(80, 24),
    );

    tokio::time::timeout(Duration::from_secs(1), async {
        while !flag.load() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("actor never started reading");

    tokio::time::timeout(Duration::from_secs(1), handle.send_input(b"x"))
        .await
        .expect("write deadlocked while read was pending")
        .expect("write observed an active read");
    handle.close().await.unwrap();
}

struct EofWithCloseFailure;

impl Transport for EofWithCloseFailure {
    fn write<'a>(&'a self, data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(std::future::ready(Ok(data.len())))
    }

    fn read<'a>(&'a self, _buf: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        Box::pin(std::future::ready(Ok(0)))
    }

    fn resize(&self, _cols: u16, _rows: u16) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn close(&self) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(std::future::ready(Err(TransportError::Backend(
            "teardown failed".into(),
        ))))
    }
}

#[tokio::test]
async fn eof_reports_teardown_failure_instead_of_clean_eof() {
    let mut handle = spawn_session(EofWithCloseFailure, Screen::new(80, 24));
    tokio::time::timeout(Duration::from_secs(1), handle.status_changed())
        .await
        .expect("status update timeout")
        .unwrap();
    assert_eq!(
        handle.status(),
        SessionStatus::Failed("backend error: teardown failed".into())
    );
}

/// A transport whose write never completes (e.g. ConPTY writer thread wedged
/// because the child stopped reading stdin). Close must still succeed by
/// abandoning the in-flight write — liveness beats delivery.
struct WedgedWriteTransport {
    closed: Arc<AtomicBool>,
    /// Set the first time the (wedged) write future is polled, proving the
    /// input was really submitted before close runs.
    write_started: Arc<AtomicBool>,
}

impl Transport for WedgedWriteTransport {
    fn write<'a>(&'a self, _data: &'a [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        let started = Arc::clone(&self.write_started);
        Box::pin(async move {
            started.store(true, Ordering::Release);
            std::future::pending::<()>().await;
            unreachable!()
        })
    }

    fn read<'a>(&'a self, buf: &'a mut [u8]) -> BoxFuture<'a, Result<usize, TransportError>> {
        // Keep the read side pending too; close must work regardless.
        let _ = &mut *buf;
        Box::pin(std::future::pending())
    }

    fn resize(&self, _cols: u16, _rows: u16) -> BoxFuture<'_, Result<(), TransportError>> {
        Box::pin(std::future::ready(Ok(())))
    }

    fn close(&self) -> BoxFuture<'_, Result<(), TransportError>> {
        self.closed.store(true, Ordering::Release);
        Box::pin(std::future::ready(Ok(())))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn close_succeeds_even_when_write_is_wedged() {
    let closed = Arc::new(AtomicBool::new(false));
    let write_started = Arc::new(AtomicBool::new(false));
    let handle = spawn_session(
        WedgedWriteTransport {
            closed: Arc::clone(&closed),
            write_started: Arc::clone(&write_started),
        },
        Screen::new(80, 24),
    );

    // Submit the input and wait until the transport's write future has really
    // been polled (so the write is genuinely wedged in flight), then close.
    // Polling `input` here is what actually drives the send_input future.
    let mut input = Box::pin(handle.send_input(b"wedged"));
    tokio::time::timeout(Duration::from_secs(1), async {
        while !write_started.load(Ordering::Acquire) {
            tokio::select! {
                result = &mut input => panic!("wedged input unexpectedly resolved: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
        }
    })
    .await
    .expect("input never reached the transport");

    // Close must complete even though the write never will. Racing the
    // abandoned write's ack is fine — close preempts it and the ack resolves
    // with an error either way.
    let mut input_done: Option<Result<usize, _>> = None;
    let close_result = tokio::time::timeout(Duration::from_secs(1), async {
        let close = handle.close();
        tokio::select! {
            result = &mut input => {
                input_done = Some(result);
                // The ack only resolves because close abandoned the write; the
                // session is now closed, so a second close reports Closed.
                Ok(())
            }
            result = close => result,
        }
    })
    .await
    .expect("close timed out while a write was wedged");
    close_result.expect("close should abandon the wedged write and succeed");
    // The status watch may lag the ack by a scheduler tick; wait for Closed.
    tokio::time::timeout(Duration::from_secs(1), async {
        while handle.status() != SessionStatus::Closed {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("status never became Closed");
    assert!(closed.load(Ordering::Acquire));

    // If we haven't observed it yet, the abandoned write's ack resolves with
    // an error rather than hanging.
    if let Some(result) = input_done {
        assert!(result.is_err(), "wedged write unexpectedly succeeded");
    } else {
        match tokio::time::timeout(Duration::from_millis(100), &mut input).await {
            Ok(Err(_)) => {}
            Ok(Ok(_)) => panic!("wedged write unexpectedly succeeded"),
            Err(_) => panic!("abandoned write ack never resolved"),
        }
    }
}

/// Dropping every handle must also terminate the actor and release the
/// transport even when a write is wedged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_all_handles_terminates_actor_despite_wedged_write() {
    let closed = Arc::new(AtomicBool::new(false));
    let write_started = Arc::new(AtomicBool::new(false));
    let handle = spawn_session(
        WedgedWriteTransport {
            closed: Arc::clone(&closed),
            write_started: Arc::clone(&write_started),
        },
        Screen::new(80, 24),
    );

    // Ensure the write is actually in flight before dropping the handle.
    // Polling `input` here is what actually drives the send_input future.
    let mut input = Box::pin(handle.send_input(b"wedged"));
    tokio::time::timeout(Duration::from_secs(1), async {
        while !write_started.load(Ordering::Acquire) {
            tokio::select! {
                result = &mut input => panic!("wedged input unexpectedly resolved: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
        }
    })
    .await
    .expect("input never reached the transport");
    std::mem::forget(input);

    drop(handle);

    // The actor must run its teardown (transport.close) after the last handle
    // is dropped; bounded close guarantees this cannot hang.
    tokio::time::timeout(Duration::from_secs(2), async {
        while !closed.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("actor did not tear down transport after all handles were dropped");
}

#[tokio::test]
async fn invalid_resize_is_rejected_before_reaching_transport() {
    let (_incoming_tx, incoming_rx) = mpsc::channel(1);
    let (writes_tx, _writes_rx) = mpsc::unbounded_channel();
    let (resizes_tx, mut resizes_rx) = mpsc::unbounded_channel();
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = spawn_session(
        PendingReadTransport {
            incoming: Arc::new(tokio::sync::Mutex::new(incoming_rx)),
            pending: Mutex::new(VecDeque::new()),
            writes: writes_tx,
            resizes: resizes_tx,
            closed,
        },
        Screen::new(80, 24),
    );

    assert!(handle.resize(0, 24).await.is_err());
    assert!(handle.resize(4096, 4096).await.is_err());
    assert!(resizes_rx.try_recv().is_err());
    handle.close().await.unwrap();
}
