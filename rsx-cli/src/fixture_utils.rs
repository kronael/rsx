use rsx_cast::as_bytes;
use rsx_cast::encode_utils::compute_crc32;
use rsx_cast::header::WalHeader;
use rsx_messages::BboRecord;
use rsx_messages::FillRecord;
use rsx_messages::OrderAcceptedRecord;
use rsx_messages::OrderCancelledRecord;
use rsx_messages::OrderDoneRecord;
use rsx_messages::OrderFailedRecord;
use rsx_messages::OrderInsertedRecord;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_types::Price;
use rsx_types::Qty;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

pub const BUY: u8 = 0;
pub const SELL: u8 = 1;
pub const SYMBOL: u32 = 1;

pub struct Wal {
    next_seq: u64,
    pub records: Vec<(u16, Vec<u8>)>,
}

impl Default for Wal {
    fn default() -> Self {
        Wal::starting_at(1)
    }
}

impl Wal {
    pub fn starting_at(seq: u64) -> Self {
        Wal {
            next_seq: seq,
            records: Vec::new(),
        }
    }

    fn take_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn push<T: Copy>(&mut self, record_type: u16, record: &T) -> &mut Self {
        self.records.push((record_type, as_bytes(record).to_vec()));
        self
    }

    pub fn set_next_seq(&mut self, seq: u64) -> &mut Self {
        self.next_seq = seq;
        self
    }

    pub fn accepted(
        &mut self,
        user_id: u32,
        order_id: u64,
        side: u8,
        price: i64,
        qty: i64,
    ) -> &mut Self {
        let record = OrderAcceptedRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            user_id,
            symbol_id: SYMBOL,
            order_id_hi: 0,
            order_id_lo: order_id,
            price,
            qty,
            side,
            tif: 0,
            reduce_only: 0,
            post_only: 0,
            cid: [0; 20],
        };
        self.push(RECORD_ORDER_ACCEPTED, &record)
    }

    pub fn inserted(
        &mut self,
        user_id: u32,
        order_id: u64,
        side: u8,
        price: i64,
        qty: i64,
    ) -> &mut Self {
        let record = OrderInsertedRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            symbol_id: SYMBOL,
            user_id,
            order_id_hi: 0,
            order_id_lo: order_id,
            price: Price(price),
            qty: Qty(qty),
            side,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };
        self.push(RECORD_ORDER_INSERTED, &record)
    }

    pub fn fill(
        &mut self,
        taker: (u32, u64),
        maker: (u32, u64),
        price: i64,
        qty: i64,
        taker_side: u8,
    ) -> &mut Self {
        let record = FillRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            symbol_id: SYMBOL,
            taker_user_id: taker.0,
            maker_user_id: maker.0,
            _pad0: 0,
            taker_order_id_hi: 0,
            taker_order_id_lo: taker.1,
            maker_order_id_hi: 0,
            maker_order_id_lo: maker.1,
            price: Price(price),
            qty: Qty(qty),
            taker_side,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
            gw_in_ns: 0,
            risk_in_ns: 0,
            me_in_ns: 0,
            match_done_ns: 0,
            gw_out_ns: 0,
        };
        self.push(RECORD_FILL, &record)
    }

    pub fn done(&mut self, user_id: u32, order_id: u64) -> &mut Self {
        let record = OrderDoneRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            symbol_id: SYMBOL,
            user_id,
            order_id_hi: 0,
            order_id_lo: order_id,
            filled_qty: Qty(0),
            remaining_qty: Qty(0),
            final_status: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
            gw_in_ns: 0,
            risk_in_ns: 0,
            me_in_ns: 0,
            match_done_ns: 0,
            gw_out_ns: 0,
        };
        self.push(RECORD_ORDER_DONE, &record)
    }

    pub fn cancelled(&mut self, user_id: u32, order_id: u64) -> &mut Self {
        let record = OrderCancelledRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            symbol_id: SYMBOL,
            user_id,
            order_id_hi: 0,
            order_id_lo: order_id,
            remaining_qty: Qty(0),
            reason: 0,
            reduce_only: 0,
            tif: 0,
            post_only: 0,
            _pad1: [0; 4],
        };
        self.push(RECORD_ORDER_CANCELLED, &record)
    }

    pub fn failed(&mut self, user_id: u32, order_id: u64, reason: u8) -> &mut Self {
        let record = OrderFailedRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            user_id,
            _pad0: 0,
            order_id_hi: 0,
            order_id_lo: order_id,
            reason,
            _pad: [0; 23],
        };
        self.push(RECORD_ORDER_FAILED, &record)
    }

    pub fn bbo(&mut self, bid_px: i64, bid_qty: i64, ask_px: i64, ask_qty: i64) -> &mut Self {
        let record = BboRecord {
            seq: self.take_seq(),
            ts_ns: 1,
            symbol_id: SYMBOL,
            _pad0: 0,
            bid_px: Price(bid_px),
            bid_qty: Qty(bid_qty),
            bid_count: 0,
            _pad1: 0,
            ask_px: Price(ask_px),
            ask_qty: Qty(ask_qty),
            ask_count: 0,
            _pad2: 0,
        };
        self.push(RECORD_BBO, &record)
    }

    pub fn raw(&mut self, record_type: u16, payload_len: usize) -> &mut Self {
        let mut payload = vec![0u8; payload_len.max(8)];
        payload[0..8].copy_from_slice(&self.take_seq().to_le_bytes());
        self.records.push((record_type, payload));
        self
    }

    pub fn write_stream(&self, wal_dir: &Path, stream_id: u32) -> PathBuf {
        let stream_dir = wal_dir.join(stream_id.to_string());
        fs::create_dir_all(&stream_dir).expect("create stream dir");
        let path = stream_dir.join(format!("{}_active.wal", stream_id));
        let mut file = File::create(&path).expect("create segment");
        for (record_type, payload) in &self.records {
            let header = WalHeader::new(*record_type, payload.len() as u16, compute_crc32(payload));
            file.write_all(header.to_bytes()).expect("write header");
            file.write_all(payload).expect("write payload");
        }
        file.sync_all().expect("sync segment");
        path
    }
}
