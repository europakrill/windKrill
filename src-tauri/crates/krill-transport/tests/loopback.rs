use krill_transport::local::{LoopbackTransport, ShellKind, SpawnOptions};
use krill_transport::Transport;

#[tokio::test]
async fn loopback_echoes_written_bytes() {
    let opts = SpawnOptions {
        shell: ShellKind::Default,
        initial_cols: 80,
        initial_rows: 24,
    };
    let t = LoopbackTransport::new(&opts);
    assert_eq!(t.size(), (80, 24));

    t.write(b"echo hi\r\n").await.unwrap();
    let mut buf = [0u8; 64];
    let n = t.read(&mut buf).await.unwrap();
    assert_eq!(&buf[..n], b"echo hi\r\n");

    // Exhausted read yields 0 bytes, not an error.
    let n = t.read(&mut buf).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn resize_is_tracked() {
    let t = LoopbackTransport::new(&SpawnOptions::default());
    t.resize(100, 40).await.unwrap();
    assert_eq!(t.size(), (100, 40));
}

#[tokio::test]
async fn invalid_resize_is_rejected_and_size_unchanged() {
    let t = LoopbackTransport::new(&SpawnOptions::default());
    assert!(t.resize(0, 24).await.is_err());
    assert!(t.resize(4096, 4096).await.is_err());
    assert_eq!(t.size(), (120, 30));
}
