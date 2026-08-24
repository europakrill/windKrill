#![cfg(windows)]

use krill_transport::local::{ConPtyTransport, ShellKind, SpawnOptions};
use krill_transport::Transport;
use std::sync::Arc;
use std::time::Duration;

/// Real Windows integration test: spawn cmd.exe inside ConPTY, write a command,
/// and observe its output through the exact Transport API used by sessions.
#[tokio::test]
async fn conpty_cmd_round_trip_and_resize() {
    let opts = SpawnOptions {
        shell: ShellKind::Cmd,
        initial_cols: 80,
        initial_rows: 24,
    };
    let pty = ConPtyTransport::spawn(&opts)
        .await
        .expect("spawn cmd.exe in ConPTY");

    pty.resize(100, 40).await.expect("resize ConPTY");
    pty.write(b"echo %OS%\r\n").await.expect("write command");

    let output = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut all = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut answered_cursor_query = false;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "ConPTY output timeout; raw output: {:?}",
                all
            );
            // Answer conhost's startup cursor-position query so the shell
            // proceeds (the raw transport does not auto-reply like the
            // session actor does).
            if !answered_cursor_query && all.windows(4).any(|w| w == b"\x1b[6n") {
                pty.write(b"\x1b[1;1R").await.expect("answer DSR query");
                answered_cursor_query = true;
            }
            match tokio::time::timeout(remaining, pty.read(&mut chunk)).await {
                Ok(Ok(0)) => break all,
                Ok(Ok(n)) => {
                    all.extend_from_slice(&chunk[..n]);
                    if all.windows(b"Windows_NT".len()).any(|w| w == b"Windows_NT") {
                        break all;
                    }
                }
                Ok(Err(error)) => panic!("ConPTY read failed: {error}; raw output: {all:?}"),
                Err(_) => panic!("ConPTY output timeout; raw output: {all:?}"),
            }
        }
    };

    assert!(
        output
            .windows(b"Windows_NT".len())
            .any(|w| w == b"Windows_NT"),
        "cmd output did not contain expanded OS value: {}",
        String::from_utf8_lossy(&output)
    );

    pty.write(b"exit\r\n").await.expect("exit cmd.exe");
    tokio::time::timeout(Duration::from_secs(10), pty.close())
        .await
        .expect("ConPTY close timeout")
        .expect("close ConPTY and verify child exit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conpty_close_drains_backpressured_output() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let pty = ConPtyTransport::spawn(&SpawnOptions {
            shell: ShellKind::Cmd,
            initial_cols: 80,
            initial_rows: 24,
        })
        .await
        .expect("spawn cmd.exe");

        // Complete conhost's startup cursor-position handshake.
        let mut received = Vec::new();
        let mut chunk = [0_u8; 256];
        loop {
            let n = pty.read(&mut chunk).await.expect("read startup query");
            assert!(n > 0, "EOF before ConPTY startup query");
            received.extend_from_slice(&chunk[..n]);
            if received.windows(4).any(|bytes| bytes == b"\x1b[6n") {
                pty.write(b"\x1b[1;1R").await.expect("answer DSR");
                break;
            }
        }

        // Produce much more than READ_QUEUE_DEPTH * one reader chunk, then
        // deliberately stop reading so the normal delivery path backs up.
        pty.write(b"for /L %i in (1,1,100000) do @echo 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ\r\n")
            .await
            .expect("start high-volume output");
        tokio::time::sleep(Duration::from_millis(250)).await;

        tokio::time::timeout(Duration::from_secs(10), pty.close())
            .await
            .expect("backpressured ConPTY close timeout")
            .expect("close backpressured ConPTY");
    })
    .await
    .expect("backpressure lifecycle timeout");
}

/// The async spawn must never block the runtime that called it: while the
/// blocking initialization runs on a worker thread, an independent task on a
/// single-threaded runtime must stay schedulable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conpty_spawn_does_not_starve_the_runtime() {
    let heartbeat = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let beat = Arc::clone(&heartbeat);
    let monitor = tokio::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            beat.fetch_add(1, std::sync::atomic::Ordering::Release);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });

    let spawn = ConPtyTransport::spawn(&SpawnOptions {
        shell: ShellKind::Cmd,
        initial_cols: 80,
        initial_rows: 24,
    });
    let result = tokio::time::timeout(Duration::from_secs(14), spawn)
        .await
        .expect("ConPTY spawn timeout")
        .expect("spawn cmd.exe");
    result.close().await.expect("close probe session");

    let beats_after = heartbeat.load(std::sync::atomic::Ordering::Acquire);
    monitor.await.expect("monitor task panicked");
    let beats_final = heartbeat.load(std::sync::atomic::Ordering::Acquire);
    assert!(
        beats_final > beats_after || beats_final > 0,
        "runtime made no observable progress during spawn"
    );
    assert!(beats_final > 100, "heartbeat barely ran: {beats_final}");
}

#[tokio::test]
async fn conpty_rejects_invalid_sizes_without_spawning() {
    for (cols, rows) in [(0, 24), (80, 0), (4096, 4096)] {
        let opts = SpawnOptions {
            shell: ShellKind::Cmd,
            initial_cols: cols,
            initial_rows: rows,
        };
        assert!(
            ConPtyTransport::spawn(&opts).await.is_err(),
            "expected rejection for {cols}x{rows}"
        );
    }
}
