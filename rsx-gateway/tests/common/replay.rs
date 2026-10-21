//! Risk's replication server as the gateway's replay sees it: a real
//! `ReplicationService` over a WAL the test wrote, behind the mandatory TLS.

use rsx_cast::CastRecord;
use rsx_cast::ReplicationService;
use rsx_cast::TlsClient;
use rsx_cast::TlsConfig;
use rsx_cast::TlsServer;
use rsx_cast::WalWriter;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const STREAM_ID: u32 = 0;

pub struct WalServer {
    pub addr: SocketAddr,
    pub tls: TlsConfig,
    pub tip_file: PathBuf,
    /// Sequence of the last record written.
    pub last_seq: u64,
    _dir: TempDir,
}

pub fn reserve_port() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    addr
}

/// A self-signed certificate used as both server identity and client CA
/// (single-box self-trust).
pub fn self_trust_tls(dir: &std::path::Path) -> TlsConfig {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .unwrap();
    let cert_path = dir.join("repl_cert.pem");
    let key_path = dir.join("repl_key.pem");
    std::fs::write(&cert_path, cert.cert.pem()).unwrap();
    std::fs::write(&key_path, cert.key_pair.serialize_pem()).unwrap();
    TlsConfig {
        server: Some(TlsServer {
            cert_path: cert_path.clone(),
            key_path,
        }),
        client: Some(TlsClient { cert_path }),
    }
}

/// Write `records` to a fresh WAL (sequences 1..=n) and serve it.
pub fn serve_wal<T: CastRecord>(records: Vec<T>) -> WalServer {
    let dir = tempfile::tempdir().unwrap();
    let wal_dir = dir.path().join("wal");
    std::fs::create_dir_all(&wal_dir).unwrap();
    let mut writer = WalWriter::new(STREAM_ID, &wal_dir, 64 * 1024 * 1024).unwrap();
    for mut record in records {
        let framed = writer.prepare(&mut record).unwrap();
        writer.append_framed(&framed).unwrap();
    }
    writer.flush().unwrap();
    let last_seq = writer.last_seq();

    let tls = self_trust_tls(dir.path());
    let addr = reserve_port();
    let server_tls = tls.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let service = ReplicationService::new(wal_dir, server_tls).unwrap();
        runtime.block_on(async move {
            service.serve(addr).await.unwrap();
        });
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while TcpStream::connect(addr).is_err() {
        assert!(
            Instant::now() < deadline,
            "replication server failed to bind"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    WalServer {
        addr,
        tls,
        tip_file: dir.path().join("gateway_replay_tip.bin"),
        last_seq,
        _dir: dir,
    }
}

/// A replication address nothing listens on.
pub fn unreachable_server() -> WalServer {
    let dir = tempfile::tempdir().unwrap();
    WalServer {
        addr: reserve_port(),
        tls: self_trust_tls(dir.path()),
        tip_file: dir.path().join("gateway_replay_tip.bin"),
        last_seq: 0,
        _dir: dir,
    }
}
