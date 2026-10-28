use crate::checkpoint::save_checkpoint;
use crate::config::apply_due_configs;
use crate::config::config_link;
use crate::config::run_config_sidecar;
use crate::config::BootConfig;
use crate::config::ScheduledConfig;
use crate::recovery::recover;
use crate::wal::CastPeer;
use rsx_book::book::Orderbook;
use rsx_cast::cast::CastSender;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_messages::decode_config_applied_record;
use rsx_types::SymbolConfig;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::net::UdpSocket;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use tokio_postgres::NoTls;

const SYMBOL_ID: u32 = 1;

fn symbol_config() -> SymbolConfig {
    SymbolConfig {
        symbol_id: SYMBOL_ID,
        price_decimals: 2,
        qty_decimals: 2,
        tick_size: 1,
        lot_size: 1,
    }
}

fn cast_sender(dir: &std::path::Path) -> (UdpSocket, CastPeer) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let sender = CastSender::new(socket.local_addr().unwrap(), SYMBOL_ID, dir).unwrap();
    (socket, CastPeer::new("test", sender))
}

/// A Postgres that completes the startup handshake and then never answers,
/// like a server whose query has stalled. Returns its port and the count of
/// bytes clients have sent after the handshake.
fn stalled_postgres() -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(AtomicUsize::new(0));
    let counter = received.clone();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut length = [0u8; 4];
        stream.read_exact(&mut length).unwrap();
        let mut startup = vec![0u8; u32::from_be_bytes(length) as usize - 4];
        stream.read_exact(&mut startup).unwrap();
        // AuthenticationOk, then ReadyForQuery.
        stream
            .write_all(&[b'R', 0, 0, 0, 8, 0, 0, 0, 0, b'Z', 0, 0, 0, 5, b'I'])
            .unwrap();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            counter.fetch_add(n, Ordering::SeqCst);
        }
    });
    (port, received)
}

async fn drive_connection(
    connection: tokio_postgres::Connection<
        tokio_postgres::Socket,
        tokio_postgres::tls::NoTlsStream,
    >,
) {
    let _ = connection.await;
}

async fn connect(port: u16) -> tokio_postgres::Client {
    let url = format!("host=127.0.0.1 port={port} user=postgres");
    let (client, connection) = tokio_postgres::connect(&url, NoTls).await.unwrap();
    tokio::spawn(drive_connection(connection));
    client
}

fn scheduled(config_version: u64, tick_size: i64) -> ScheduledConfig {
    ScheduledConfig {
        config: SymbolConfig {
            tick_size,
            ..symbol_config()
        },
        config_version,
        effective_at_ms: 1000 + config_version,
    }
}

#[test]
fn due_configs_are_applied_in_order_logged_and_reported_back() {
    let wal_dir = tempfile::tempdir().unwrap();
    let mut wal_writer = WalWriter::new(SYMBOL_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap();
    let (_risk, mut risk_sender) = cast_sender(wal_dir.path());
    let (_mkt, mut mkt_sender) = cast_sender(wal_dir.path());
    let mut book = Orderbook::new(symbol_config(), 16, 100);
    let (mut loop_end, mut sidecar_end) = config_link();
    sidecar_end.due.push(scheduled(1, 5)).unwrap();
    sidecar_end.due.push(scheduled(2, 10)).unwrap();

    let applied = apply_due_configs(
        &mut loop_end,
        &mut book,
        &mut wal_writer,
        &mut risk_sender,
        &mut mkt_sender,
        SYMBOL_ID,
    );
    assert_eq!(applied, Some(2));
    // Nothing is due now: a second call writes no record.
    let applied = apply_due_configs(
        &mut loop_end,
        &mut book,
        &mut wal_writer,
        &mut risk_sender,
        &mut mkt_sender,
        SYMBOL_ID,
    );
    assert_eq!(applied, None);
    wal_writer.flush().unwrap();

    assert_eq!(book.config.tick_size, 10);
    let mut reader = WalReader::open_from_seq(SYMBOL_ID, 1, wal_dir.path()).unwrap();
    for (seq, version) in [(1, 1), (2, 2)] {
        let raw = reader.next().unwrap().unwrap();
        let record = decode_config_applied_record(&raw.payload).unwrap();
        assert_eq!((record.seq, record.config_version), (seq, version));
        assert_eq!(record.effective_at_ms, 1000 + version);
    }
    assert!(reader.next().unwrap().is_none());
    for version in [1, 2] {
        let (applied, applied_at_ns) = sidecar_end.applied.pop().unwrap();
        assert_eq!(applied.config_version, version);
        assert!(applied_at_ns > 0);
    }
    assert!(sidecar_end.applied.pop().is_err());
}

#[test]
fn a_checkpoint_taken_after_an_apply_carries_the_new_tick_into_recovery() {
    let wal_dir = tempfile::tempdir().unwrap();
    let wal_path = wal_dir.path().to_str().unwrap();
    let mut wal_writer = WalWriter::new(SYMBOL_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap();
    let (_risk, mut risk_sender) = cast_sender(wal_dir.path());
    let (_mkt, mut mkt_sender) = cast_sender(wal_dir.path());
    let mut book = Orderbook::new(symbol_config(), 16, 100);
    let (mut loop_end, mut sidecar_end) = config_link();
    sidecar_end.due.push(scheduled(2, 10)).unwrap();
    apply_due_configs(
        &mut loop_end,
        &mut book,
        &mut wal_writer,
        &mut risk_sender,
        &mut mkt_sender,
        SYMBOL_ID,
    );
    wal_writer.flush().unwrap();
    save_checkpoint(&book, wal_path, SYMBOL_ID, wal_writer.last_seq()).unwrap();

    // The write of the new applied row failed: the boot config is the old one.
    let stale = BootConfig {
        config: symbol_config(),
        applied_version: Some(1),
        book_capacity: 16,
    };
    let recovered = recover(stale, wal_path, 1).unwrap();
    assert_eq!(recovered.book.config.tick_size, 10);
    assert_eq!(recovered.checkpoint_wal_seq, 1);
    assert_eq!(recovered.next_seq, 2);
}

#[test]
fn loop_side_config_step_does_not_wait_on_a_stalled_postgres() {
    let (port, received) = stalled_postgres();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = rt.block_on(connect(port));
    let (mut loop_end, sidecar_end) = config_link();
    let sidecar = std::thread::spawn(move || {
        rt.block_on(run_config_sidecar(
            client,
            SYMBOL_ID,
            0,
            Duration::from_millis(10),
            sidecar_end,
        ))
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while received.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the sidecar never queried");
        std::thread::sleep(Duration::from_millis(5));
    }
    // The poll query is on the wire and unanswered.

    let wal_dir = tempfile::tempdir().unwrap();
    let mut wal_writer = WalWriter::new(SYMBOL_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap();
    let (_risk, mut risk_sender) = cast_sender(wal_dir.path());
    let (_mkt, mut mkt_sender) = cast_sender(wal_dir.path());
    let mut book = Orderbook::new(symbol_config(), 16, 100);
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for _ in 0..10_000 {
            apply_due_configs(
                &mut loop_end,
                &mut book,
                &mut wal_writer,
                &mut risk_sender,
                &mut mkt_sender,
                SYMBOL_ID,
            );
        }
        let _ = done.send(loop_end);
    });

    let loop_end = finished
        .recv_timeout(Duration::from_secs(2))
        .expect("the loop-side config step waited on Postgres");
    assert!(
        !sidecar.is_finished(),
        "the poll query should still be stalled"
    );
    drop(loop_end);
}

#[test]
fn book_capacity_defaults_and_rejects_bad_values() {
    use crate::config::book_capacity;
    use crate::recovery::BOOK_ORDER_CAPACITY;
    assert_eq!(book_capacity(None).unwrap(), BOOK_ORDER_CAPACITY);
    assert_eq!(book_capacity(Some("65536")).unwrap(), 65_536);
    assert_eq!(book_capacity(Some(" 1 ")).unwrap(), 1);
    assert_eq!(book_capacity(Some("4294967294")).unwrap(), u32::MAX - 1);
    for bad in [
        "",
        "0",
        "-1",
        "1e6",
        "1_000",
        "4294967295",
        "4294967296",
        "many",
    ] {
        let err = book_capacity(Some(bad)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{bad}");
        assert!(
            err.to_string().contains("RSX_ME_BOOK_CAPACITY"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn config_poll_interval_defaults_and_rejects_bad_values() {
    use crate::config::config_poll_interval;
    use crate::config::CONFIG_POLL_INTERVAL;
    assert_eq!(config_poll_interval(None).unwrap(), CONFIG_POLL_INTERVAL);
    assert_eq!(CONFIG_POLL_INTERVAL, Duration::from_secs(10));
    assert_eq!(
        config_poll_interval(Some("1")).unwrap(),
        Duration::from_secs(1)
    );
    assert_eq!(
        config_poll_interval(Some(" 30 ")).unwrap(),
        Duration::from_secs(30)
    );
    for bad in ["", "0", "-1", "1.5", "1e3", "4294967296", "soon"] {
        let err = config_poll_interval(Some(bad)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{bad}");
        assert!(
            err.to_string().contains("RSX_ME_CONFIG_POLL_S"),
            "{bad}: {err}"
        );
    }
}
