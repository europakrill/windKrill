use krill_core::Screen;
use krill_session::Session;
use krill_transport::local::{LoopbackTransport, SpawnOptions};

#[tokio::test]
async fn pump_moves_written_bytes_into_screen() {
    let opts = SpawnOptions {
        initial_cols: 40,
        initial_rows: 10,
        ..Default::default()
    };
    let transport = LoopbackTransport::new(&opts);
    let mut session = Session::new(transport, Screen::new(40, 10));

    // Simulate the shell writing output (loopback echoes our "input").
    session
        .send_input(b"\x1b[2J\x1b[Huptime\r\n")
        .await
        .unwrap();
    let n = session.pump().await.unwrap();
    // Loopback may echo everything in one chunk; only require progress.
    assert!(n > 0);
    assert!(session.screen().text().contains("uptime"));
}

#[tokio::test]
async fn empty_pump_is_zero_not_error() {
    let opts = SpawnOptions::default();
    let transport = LoopbackTransport::new(&opts);
    let mut session = Session::new(transport, Screen::new(80, 24));
    assert_eq!(session.pump().await.unwrap(), 0);
}
