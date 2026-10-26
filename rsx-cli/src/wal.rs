use crate::output;
use crate::output::Failure;
use crate::output::Format;
use crate::output::Outcome;
use crate::record;
use rsx_cast::header::WalHeader;
use rsx_cast::wal::extract_seq;
use rsx_cast::wal::RawWalRecord;
use rsx_cast::wal::WalReader;
use rsx_messages::RECORD_BBO;
use rsx_messages::RECORD_CANCEL_REQUEST;
use rsx_messages::RECORD_CONFIG_APPLIED;
use rsx_messages::RECORD_FILL;
use rsx_messages::RECORD_LIQUIDATION;
use rsx_messages::RECORD_MARK_PRICE;
use rsx_messages::RECORD_ORDER_ACCEPTED;
use rsx_messages::RECORD_ORDER_CANCELLED;
use rsx_messages::RECORD_ORDER_DONE;
use rsx_messages::RECORD_ORDER_FAILED;
use rsx_messages::RECORD_ORDER_INSERTED;
use rsx_messages::RECORD_ORDER_REQUEST;
use rsx_messages::RECORD_ORDER_RESPONSE;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[cfg(test)]
#[path = "wal_test.rs"]
mod wal_test;

const FOLLOW_POLL: Duration = Duration::from_millis(100);

/// A stream's WAL directory and the first sequence number to emit.
pub struct Stream {
    pub stream_id: u32,
    pub wal_dir: PathBuf,
    pub from_seq: u64,
}

/// Record selection; every set criterion must match.
pub struct Filters {
    record_types: Vec<u16>,
    symbol: Option<u32>,
    user: Option<u32>,
    from_ts: Option<u64>,
    to_ts: Option<u64>,
}

impl Filters {
    /// Fails with `Misuse` on a record type name that is not known.
    pub fn new(
        record_types: &[String],
        symbol: Option<u32>,
        user: Option<u32>,
        from_ts: Option<u64>,
        to_ts: Option<u64>,
    ) -> Result<Self, Failure> {
        let mut types = Vec::with_capacity(record_types.len());
        for name in record_types {
            match record::name_to_record_type(name) {
                Some(record_type) => types.push(record_type),
                None => return Err(Failure::misuse(format!("unknown record type: {}", name))),
            }
        }
        Ok(Filters {
            record_types: types,
            symbol,
            user,
            from_ts,
            to_ts,
        })
    }

    pub fn matches(&self, record_type: u16, payload: &[u8]) -> bool {
        if !self.record_types.is_empty() && !self.record_types.contains(&record_type) {
            return false;
        }
        let ts_ns = extract_ts_ns(payload);
        if let (Some(from), Some(ts_ns)) = (self.from_ts, ts_ns) {
            if ts_ns < from {
                return false;
            }
        }
        if let (Some(to), Some(ts_ns)) = (self.to_ts, ts_ns) {
            if ts_ns > to {
                return false;
            }
        }
        if let Some(symbol) = self.symbol {
            if extract_symbol_id(record_type, payload) != Some(symbol) {
                return false;
            }
        }
        if let Some(user) = self.user {
            if extract_user_id(record_type, payload) != Some(user) {
                return false;
            }
        }
        true
    }
}

fn extract_ts_ns(payload: &[u8]) -> Option<u64> {
    let bytes = payload.get(8..16)?;
    Some(u64::from_le_bytes(bytes.try_into().ok()?))
}

fn read_u32_le(payload: &[u8], offset: usize) -> Option<u32> {
    let bytes = payload.get(offset..offset + 4)?;
    Some(u32::from_le_bytes(bytes.try_into().ok()?))
}

fn extract_symbol_id(record_type: u16, payload: &[u8]) -> Option<u32> {
    match record_type {
        RECORD_FILL
        | RECORD_BBO
        | RECORD_ORDER_INSERTED
        | RECORD_ORDER_CANCELLED
        | RECORD_ORDER_DONE
        | RECORD_CONFIG_APPLIED
        | RECORD_MARK_PRICE => read_u32_le(payload, 16),
        RECORD_ORDER_ACCEPTED
        | RECORD_CANCEL_REQUEST
        | RECORD_LIQUIDATION
        | RECORD_ORDER_RESPONSE => read_u32_le(payload, 20),
        RECORD_ORDER_REQUEST => read_u32_le(payload, 12),
        _ => None,
    }
}

fn extract_user_id(record_type: u16, payload: &[u8]) -> Option<u32> {
    match record_type {
        RECORD_ORDER_INSERTED | RECORD_ORDER_CANCELLED | RECORD_ORDER_DONE | RECORD_FILL => {
            read_u32_le(payload, 20)
        }
        RECORD_ORDER_ACCEPTED
        | RECORD_ORDER_FAILED
        | RECORD_CANCEL_REQUEST
        | RECORD_LIQUIDATION
        | RECORD_ORDER_RESPONSE => read_u32_le(payload, 16),
        RECORD_ORDER_REQUEST => read_u32_le(payload, 8),
        _ => None,
    }
}

fn open_reader(stream_id: u32, from_seq: u64, wal_dir: &Path) -> Result<WalReader, Failure> {
    WalReader::open_from_seq(stream_id, from_seq, wal_dir)
        .map_err(|err| Failure::runtime(format!("failed to open wal: {}", err)))
}

fn next_record(reader: &mut WalReader) -> Result<Option<RawWalRecord>, Failure> {
    reader
        .next()
        .map_err(|err| Failure::runtime(format!("wal read error: {}", err)))
}

fn is_before(raw: &RawWalRecord, from_seq: u64) -> bool {
    extract_seq(&raw.payload).is_some_and(|seq| seq < from_seq)
}

/// Prints every matching record of the stream from `from_seq` on, then
/// `total: N records` on stderr. A corrupt record ends the dump with a
/// `Runtime` failure after the records before it were printed.
pub fn dump(stream: &Stream, filters: &Filters, format: &Format) -> Outcome {
    let mut reader = open_reader(stream.stream_id, stream.from_seq, &stream.wal_dir)?;
    let mut count: u64 = 0;
    while let Some(raw) = next_record(&mut reader)? {
        if is_before(&raw, stream.from_seq) {
            continue;
        }
        if !filters.matches(raw.header.record_type, &raw.payload) {
            continue;
        }
        output::print_record(format, &raw);
        count += 1;
    }
    eprintln!("total: {} records", count);
    Ok(())
}

/// Prints the matching records from `from_seq` on, then polls for more until
/// Ctrl-C. Each record is emitted once: every poll re-opens the reader and
/// skips what was already printed. A read error ends the follow with the
/// error on stderr and `Ok`.
pub fn follow(stream: &Stream, filters: &Filters, format: &Format) -> Outcome {
    let running = install_ctrlc_handler()?;
    let mut next_seq = stream.from_seq;
    let mut reader = open_reader(stream.stream_id, next_seq, &stream.wal_dir)?;

    while running.load(Ordering::SeqCst) {
        match reader.next() {
            Ok(Some(raw)) => {
                let seq = extract_seq(&raw.payload).unwrap_or(0);
                if seq < next_seq {
                    continue;
                }
                next_seq = seq.saturating_add(1);
                if filters.matches(raw.header.record_type, &raw.payload) {
                    output::print_record(format, &raw);
                }
            }
            Ok(None) => {
                thread::sleep(FOLLOW_POLL);
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                reader = match WalReader::open_from_seq(stream.stream_id, next_seq, &stream.wal_dir)
                {
                    Ok(reopened) => reopened,
                    Err(err) => {
                        eprintln!("wal reopen error: {}", err);
                        thread::sleep(FOLLOW_POLL);
                        continue;
                    }
                };
            }
            Err(err) => {
                eprintln!("wal read error: {}", err);
                break;
            }
        }
    }
    Ok(())
}

fn install_ctrlc_handler() -> Result<Arc<AtomicBool>, Failure> {
    let running = Arc::new(AtomicBool::new(true));
    let flag = running.clone();
    ctrlc::set_handler(move || flag.store(false, Ordering::SeqCst))
        .map_err(|err| Failure::runtime(format!("failed to install signal handler: {}", err)))?;
    Ok(running)
}

/// Prints the count of matching records per type and the total, from
/// `from_seq` on.
pub fn stats(stream: &Stream, filters: &Filters) -> Outcome {
    let mut reader = open_reader(stream.stream_id, stream.from_seq, &stream.wal_dir)?;
    let mut counts: BTreeMap<&'static str, u64> = BTreeMap::new();
    while let Some(raw) = next_record(&mut reader)? {
        if is_before(&raw, stream.from_seq) {
            continue;
        }
        let record_type = raw.header.record_type;
        if filters.matches(record_type, &raw.payload) {
            *counts.entry(record::record_name(record_type)).or_insert(0) += 1;
        }
    }
    for (name, count) in &counts {
        println!("{:<20} {}", name, count);
    }
    println!("{:<20} {}", "total", counts.values().sum::<u64>());
    Ok(())
}

/// Prints each record of one segment file as a JSON line, without CRC
/// checks, up to the first partial record; `total: N records` goes to stderr.
pub fn dump_file(path: &Path) -> Outcome {
    let mut file = File::open(path)
        .map_err(|err| Failure::runtime(format!("failed to open {}: {}", path.display(), err)))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|err| Failure::runtime(format!("failed to read {}: {}", path.display(), err)))?;

    let format = Format::Json;
    let mut offset = 0;
    let mut count: u64 = 0;
    while let Some(header) = buf.get(offset..offset + WalHeader::SIZE) {
        let record_type = u16::from_le_bytes([header[2], header[3]]);
        let len = u16::from_le_bytes([header[4], header[5]]);
        let crc32 = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let start = offset + WalHeader::SIZE;
        let Some(payload) = buf.get(start..start + len as usize) else {
            break;
        };
        let raw = RawWalRecord {
            header: WalHeader::new(record_type, len, crc32),
            payload: payload.to_vec(),
        };
        output::print_record(&format, &raw);
        offset = start + len as usize;
        count += 1;
    }
    eprintln!("total: {} records", count);
    Ok(())
}
