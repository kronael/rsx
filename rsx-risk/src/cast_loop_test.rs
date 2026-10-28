use crate::cast_loop::bind;
use rsx_cast::config::CastConfig;
use rsx_cast::wal::oldest_and_highest_seq;
use rsx_messages::CancelRequest;
use rsx_risk::config::MeStreamConfig;
use rsx_risk::config::RuntimeConfig;
use rsx_risk::rings::EgressStream;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::net::UdpSocket;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use tempfile::TempDir;

const SYMBOL_ID: u32 = 3;

fn cancel(order: u64) -> CancelRequest {
    CancelRequest {
        seq: 0,
        ts_ns: 1_000_000_000 + order,
        user_id: 10,
        symbol_id: SYMBOL_ID,
        order_id_hi: 0,
        order_id_lo: order,
        _pad: [0; 24],
    }
}

fn free_addrs(count: usize) -> Vec<SocketAddr> {
    let sockets: Vec<UdpSocket> = (0..count)
        .map(|_| UdpSocket::bind("127.0.0.1:0").unwrap())
        .collect();
    sockets
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect()
}

fn shard_config(wal_dir: &TempDir) -> RuntimeConfig {
    let addrs = free_addrs(4);
    RuntimeConfig {
        db_url: String::new(),
        wal_dir: wal_dir.path().to_path_buf(),
        me_streams: vec![MeStreamConfig {
            symbol_id: SYMBOL_ID,
            cast_addr: addrs[0],
            recv_addr: addrs[1],
            replay_addr: String::new(),
            send_bind: None,
            sender_addr: addrs[0],
        }],
        mark_addr: addrs[2],
        mark_sender_addr: addrs[0],
        mark_replay_addr: None,
        risk_addr: addrs[3],
        gw_addr: addrs[0],
        gw_replay_addr: None,
        gw_sender_addr: addrs[0],
        gw_send_bind: None,
        replication_bind_addr: None,
        core_id: None,
    }
}

#[test]
fn bind_waits_for_a_busy_port_to_free() {
    let wal_dir = TempDir::new().unwrap();
    let config = shard_config(&wal_dir);
    let holder = UdpSocket::bind(config.risk_addr).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        drop(holder);
    });

    let started = Instant::now();
    let deadline = started + Duration::from_secs(5);
    let bound = bind(&config, &[], deadline, &AtomicBool::new(false));
    release.join().unwrap();

    assert!(bound.is_ok());
    assert!(started.elapsed() >= Duration::from_millis(250));
}

#[test]
fn bind_past_the_deadline_names_the_busy_address() {
    let wal_dir = TempDir::new().unwrap();
    let config = shard_config(&wal_dir);
    let _holder = UdpSocket::bind(config.risk_addr).unwrap();

    let deadline = Instant::now() + Duration::from_millis(500);
    let bound = bind(&config, &[], deadline, &AtomicBool::new(false));

    let Err(error) = &bound else {
        panic!("bind succeeded on a held port");
    };
    assert_eq!(error.kind(), ErrorKind::AddrInUse);
    assert!(error.to_string().contains(&config.risk_addr.to_string()));
}

#[test]
fn shutdown_ends_the_wait_for_a_busy_port() {
    let wal_dir = TempDir::new().unwrap();
    let config = shard_config(&wal_dir);
    let _holder = UdpSocket::bind(config.risk_addr).unwrap();
    let shutdown = AtomicBool::new(false);
    shutdown.store(true, Ordering::Relaxed);

    let deadline = Instant::now() + Duration::from_secs(30);
    let bound = bind(&config, &[], deadline, &shutdown);

    let Err(error) = &bound else {
        panic!("bind succeeded on a held port");
    };
    assert_eq!(error.kind(), ErrorKind::Interrupted);
}

#[test]
fn me_receiver_resumes_after_the_warm_tip() {
    let wal_dir = TempDir::new().unwrap();
    let config = shard_config(&wal_dir);
    let mut tips = vec![0u64; 8];
    tips[SYMBOL_ID as usize] = 41;

    let deadline = Instant::now() + Duration::from_secs(5);
    let bound = bind(&config, &tips, deadline, &AtomicBool::new(false));

    let Ok(io) = &bound else {
        panic!("bind failed on free ports");
    };
    assert_eq!(io.me_receivers[0].receiver.expected_seq(), 42);
}

#[test]
fn me_stream_continues_the_previous_mains_numbering() {
    let wal_dir = TempDir::new().unwrap();
    let config = shard_config(&wal_dir);
    let me_wal_dir = wal_dir.path().join("me");
    let sink = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut previous = EgressStream::open(
        SYMBOL_ID,
        sink.local_addr().unwrap(),
        &me_wal_dir,
        &CastConfig::default(),
    )
    .unwrap();
    for id in 1..=3 {
        previous.forward(&mut cancel(id)).unwrap();
    }
    previous.flush().unwrap();
    drop(previous);

    let deadline = Instant::now() + Duration::from_secs(5);
    let bound = bind(&config, &[], deadline, &AtomicBool::new(false));

    let Ok(mut io) = bound else {
        panic!("bind failed on free ports");
    };
    let stream = io.me_streams.get_mut(&SYMBOL_ID).unwrap();
    stream.forward(&mut cancel(4)).unwrap();
    stream.flush().unwrap();
    let range = oldest_and_highest_seq(SYMBOL_ID, &me_wal_dir).unwrap();
    assert_eq!(range, Some((1, 4)));
}
