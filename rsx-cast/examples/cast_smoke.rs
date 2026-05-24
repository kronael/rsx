use rsx_cast::cast::CastRecv;
use rsx_cast::cast::CastReceiver;
use rsx_cast::cast::CastSender;
use rsx_messages::FillRecord;
use rsx_types::Price;
use rsx_types::Qty;
use std::net::SocketAddr;
use std::net::UdpSocket;

fn pick_port() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap()
}

fn main() {
    let tmp = tempfile::TempDir::new().unwrap();
    let recv_addr = pick_port();
    let send_bind = pick_port();
    eprintln!("recv on {recv_addr}, send from {send_bind}");

    let mut sender = CastSender::with_config(
        recv_addr, 1, tmp.path(),
        &rsx_cast::config::CastConfig {
            sender_bind_addr: Some(send_bind.to_string()),
            ..Default::default()
        },
    ).unwrap();
    let actual_send_addr = sender.local_addr().unwrap();
    eprintln!("sender bound at {actual_send_addr}");

    let mut receiver = CastReceiver::new(recv_addr, actual_send_addr).unwrap();
    eprintln!("receiver created");

    let mut rec = FillRecord {
        seq: 0, ts_ns: 0, symbol_id: 1, taker_user_id: 1, maker_user_id: 2,
        _pad0: 0, taker_order_id_hi: 0, taker_order_id_lo: 200,
        maker_order_id_hi: 0, maker_order_id_lo: 100,
        price: Price(50_000), qty: Qty(100),
        taker_side: 0, reduce_only: 0, tif: 0, post_only: 0,
        _pad1: [0; 4], taker_ts_ns: 0,
    };
    eprintln!("sending...");
    sender.send(&mut rec).unwrap();
    eprintln!("send returned");

    // Try to recv with retries
    for i in 0..100 {
        if let CastRecv::Data(hdr, data) = receiver.try_recv() {
            eprintln!("recv ok after {i} attempts: type={} len={}", hdr.record_type, data.len());
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!("FAILED: never received");
    std::process::exit(1);
}
