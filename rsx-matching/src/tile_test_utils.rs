//! Fixture for the ME tile tests: the book, WAL writer, dedup tracker and
//! both cast senders are the production types; the risk and marketdata ends
//! are UDP sockets bound on localhost and the WAL lives in a temp dir.

use crate::cancel::process_cancel;
use crate::order::process_order;
use rsx_book::book::Orderbook;
use rsx_cast::cast::CastSender;
use rsx_cast::decode_payload;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::WalReader;
use rsx_cast::wal::WalWriter;
use rsx_cast::WalHeader;
use rsx_health::LoadGauges;
use rsx_matching::dedup::DedupTracker;
use rsx_matching::wal::CastPeer;
use rsx_matching::wal::OrderKey;
use rsx_messages::CancelRequest;
use rsx_messages::OrderMessage;
use rsx_types::SymbolConfig;
use rustc_hash::FxHashMap;
use std::net::UdpSocket;
use std::sync::Arc;
use tempfile::TempDir;

pub const SYMBOL_ID: u32 = 1;
pub const MID_PRICE: i64 = 50_000;
pub const BUY: u8 = 0;
pub const SELL: u8 = 1;
pub const GTC: u8 = 0;
pub const GATEWAY_IN_NS: u64 = 555;
pub const RISK_IN_NS: u64 = 777;

/// One framed record as it crossed a stream: record type and payload bytes.
pub type Frame = (u16, Vec<u8>);

/// The records the tile produced since the previous `take`, per stream.
pub struct Output {
    pub wal: Vec<Frame>,
    pub risk: Vec<Frame>,
    pub mkt: Vec<Frame>,
}

impl Output {
    /// Every record is on all three streams, byte for byte (one seq, one CRC).
    pub fn streams_agree(&self) -> &[Frame] {
        assert_eq!(self.risk, self.wal, "risk stream differs from the WAL");
        assert_eq!(self.mkt, self.wal, "marketdata stream differs from the WAL");
        &self.wal
    }
}

pub struct Tile {
    pub book: Orderbook,
    pub order_index: FxHashMap<OrderKey, u32>,
    wal_writer: WalWriter,
    cast_sender: CastPeer,
    mkt_sender: CastPeer,
    dedup: DedupTracker,
    gauges: Arc<LoadGauges>,
    risk_socket: UdpSocket,
    mkt_socket: UdpSocket,
    wal_dir: TempDir,
    wal_taken: usize,
}

impl Tile {
    pub fn new() -> Self {
        let config = SymbolConfig {
            symbol_id: SYMBOL_ID,
            price_decimals: 0,
            qty_decimals: 0,
            tick_size: 1,
            lot_size: 1,
        };
        let wal_dir = tempfile::tempdir().unwrap();
        let wal_writer = WalWriter::new(SYMBOL_ID, wal_dir.path(), 64 * 1024 * 1024).unwrap();
        let risk_socket = bind_receiver();
        let mkt_socket = bind_receiver();
        let cast_sender = CastPeer::new(
            "risk",
            CastSender::new(risk_socket.local_addr().unwrap(), SYMBOL_ID, wal_dir.path()).unwrap(),
        );
        let mkt_sender = CastPeer::new(
            "marketdata",
            CastSender::new(mkt_socket.local_addr().unwrap(), SYMBOL_ID, wal_dir.path()).unwrap(),
        );
        Self {
            book: Orderbook::new(config, 1024, MID_PRICE),
            order_index: FxHashMap::default(),
            wal_writer,
            cast_sender,
            mkt_sender,
            dedup: DedupTracker::new(),
            gauges: LoadGauges::new(),
            risk_socket,
            mkt_socket,
            wal_dir,
            wal_taken: 0,
        }
    }

    pub fn submit(&mut self, message: &OrderMessage) {
        process_order(
            &mut self.book,
            &mut self.wal_writer,
            &mut self.cast_sender,
            &mut self.mkt_sender,
            &mut self.order_index,
            &mut self.dedup,
            &self.gauges,
            SYMBOL_ID,
            message,
        );
    }

    pub fn cancel(&mut self, user_id: u32, order_id_lo: u64) {
        let request = CancelRequest {
            seq: 0,
            ts_ns: 0,
            user_id,
            symbol_id: SYMBOL_ID,
            order_id_hi: 0,
            order_id_lo,
            _pad: [0; 24],
        };
        process_cancel(
            &mut self.book,
            &mut self.wal_writer,
            &mut self.cast_sender,
            &mut self.mkt_sender,
            &mut self.order_index,
            SYMBOL_ID,
            &request,
        );
    }

    /// Records produced since the previous call. Loopback UDP is delivered
    /// before `send_to` returns, so draining the non-blocking sockets is
    /// complete and deterministic.
    pub fn take(&mut self) -> Output {
        self.wal_writer.flush().unwrap();
        let mut reader = WalReader::open_from_seq(SYMBOL_ID, 1, self.wal_dir.path()).unwrap();
        let mut all = Vec::new();
        while let Some(raw) = reader.next().unwrap() {
            all.push((raw.header.record_type, raw.payload));
        }
        let wal = all.split_off(self.wal_taken);
        self.wal_taken += wal.len();
        Output {
            wal,
            risk: drain(&self.risk_socket),
            mkt: drain(&self.mkt_socket),
        }
    }
}

fn bind_receiver() -> UdpSocket {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.set_nonblocking(true).unwrap();
    socket
}

fn drain(socket: &UdpSocket) -> Vec<Frame> {
    let mut frames = Vec::new();
    let mut buf = [0u8; 2048];
    while let Ok(len) = socket.recv(&mut buf) {
        let header = WalHeader::from_bytes(&buf[..len]).unwrap();
        frames.push((header.record_type, buf[WalHeader::SIZE..len].to_vec()));
    }
    frames
}

pub fn order_message(
    user_id: u32,
    order_id_lo: u64,
    side: u8,
    price: i64,
    qty: i64,
) -> OrderMessage {
    OrderMessage {
        seq: 0,
        price,
        qty,
        side,
        tif: GTC,
        reduce_only: 0,
        post_only: 0,
        _pad1: [0; 4],
        user_id,
        _pad2: 0,
        timestamp_ns: GATEWAY_IN_NS,
        order_id_hi: 0,
        order_id_lo,
        risk_in_ns: RISK_IN_NS,
    }
}

pub fn record_types(frames: &[Frame]) -> Vec<u16> {
    frames.iter().map(|(record_type, _)| *record_type).collect()
}

pub fn record_seqs(frames: &[Frame]) -> Vec<u64> {
    frames
        .iter()
        .map(|(_, payload)| extract_seq(payload).unwrap())
        .collect()
}

/// The records of `record_type`, decoded, in stream order.
pub fn decode_all<T: Copy>(frames: &[Frame], record_type: u16) -> Vec<T> {
    frames
        .iter()
        .filter(|(frame_type, _)| *frame_type == record_type)
        .map(|(_, payload)| decode_payload::<T>(payload).unwrap())
        .collect()
}

/// Leave non-zero bytes in the stack space a following call will reuse.
#[inline(never)]
pub fn dirty_stack() {
    let bytes = [0xAAu8; 32 * 1024];
    std::hint::black_box(&bytes);
}
