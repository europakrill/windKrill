#![cfg(windows)]

use krill_core::Screen;
use krill_session::spawn_session;
use krill_transport::local::{ConPtyTransport, ShellKind, SpawnOptions};
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn conpty_output_reaches_vt_parser_and_screen() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let options = SpawnOptions {
            shell: ShellKind::Cmd,
            initial_cols: 80,
            initial_rows: 24,
        };
        let transport = ConPtyTransport::spawn(&options)
            .await
            .expect("spawn cmd.exe in ConPTY");
        let mut session = spawn_session(transport, Screen::new(80, 24));

        session.resize(100, 40).await.expect("resize session");
        session
            .send_input(b"echo %OS%\r\n")
            .await
            .expect("send cmd command");

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let text = session.screen().text();
            if text.contains("Windows_NT") {
                assert_eq!(
                    (session.screen().cols(), session.screen().rows()),
                    (100, 40)
                );
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(!remaining.is_zero(), "screen timeout; text: {text:?}");
            tokio::time::timeout(remaining, session.screen_changed())
                .await
                .expect("screen update timeout")
                .expect("session actor stopped");
        }

        session.close().await.expect("close ConPTY session");
    })
    .await
    .expect("ConPTY actor lifecycle timeout");
}
