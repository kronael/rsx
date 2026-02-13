use clap::Parser;
use clap::Subcommand;
use rsx_dxs::records::*;
use rsx_dxs::wal::extract_seq;
use rsx_dxs::wal::WalReader;
use serde_json::json;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "rsxcli", about = "RSX CLI tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Dump WAL records for a stream
    WalDump {
        /// Stream ID to read
        stream_id: u32,
        /// WAL directory path
        wal_dir: PathBuf,
        /// Start from this sequence number
        #[arg(default_value = "0")]
        from_seq: u64,
        /// Output as JSON lines (default: text)
        #[arg(long)]
        json: bool,
    },
    /// Dump a single WAL file as JSON lines
    Dump {
        /// WAL file path
        file: PathBuf,
    },
}

fn record_name(rt: u16) -> &'static str {
    match rt {
        RECORD_FILL => "FILL",
        RECORD_BBO => "BBO",
        RECORD_ORDER_INSERTED => "ORDER_INSERTED",
        RECORD_ORDER_CANCELLED => "ORDER_CANCELLED",
        RECORD_ORDER_DONE => "ORDER_DONE",
        RECORD_CONFIG_APPLIED => "CONFIG_APPLIED",
        RECORD_CAUGHT_UP => "CAUGHT_UP",
        RECORD_ORDER_ACCEPTED => "ORDER_ACCEPTED",
        RECORD_MARK_PRICE => "MARK_PRICE",
        RECORD_ORDER_REQUEST => "ORDER_REQUEST",
        RECORD_ORDER_RESPONSE => "ORDER_RESPONSE",
        RECORD_CANCEL_REQUEST => "CANCEL_REQUEST",
        RECORD_ORDER_FAILED => "ORDER_FAILED",
        _ => "UNKNOWN",
    }
}

fn wal_dump(
    stream_id: u32,
    wal_dir: PathBuf,
    from_seq: u64,
    json: bool,
) {
    let mut reader = WalReader::open_from_seq(
        stream_id, from_seq, &wal_dir,
    )
    .expect("failed to open wal");

    if json {
        dump_json(&mut reader);
    } else {
        dump_text(&mut reader);
    }
}

fn dump_text(reader: &mut WalReader) {
    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload).unwrap_or(0);

        println!(
            "seq={:<8} type={:<18} len={:<4} crc=0x{:08x}",
            seq,
            record_name(rt),
            len,
            raw.header.crc32,
        );
        count += 1;
    }
    eprintln!("total: {} records", count);
}

fn dump_json(reader: &mut WalReader) {
    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        let len = raw.header.len;
        let seq = extract_seq(&raw.payload).unwrap_or(0);

        let obj = json!({
            "seq": seq,
            "type": record_name(rt),
            "len": len,
            "crc32": format!("0x{:08x}", raw.header.crc32),
        });
        println!("{}", obj);
        count += 1;
    }
    eprintln!("total: {} records", count);
}

fn dump_file(file: PathBuf) {
    use std::fs::File;
    use std::io::Read;

    let mut f = File::open(&file).expect("failed to open file");
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).expect("failed to read file");

    let mut offset = 0;
    let mut count = 0;

    while offset + 16 <= buf.len() {
        let header = &buf[offset..offset+16];
        let len = u32::from_le_bytes([
            header[4], header[5], header[6], header[7]
        ]) as usize;
        let rt = u16::from_le_bytes([header[8], header[9]]);
        let crc32 = u32::from_le_bytes([
            header[12], header[13], header[14], header[15]
        ]);

        if offset + 16 + len > buf.len() {
            break;
        }

        let payload = &buf[offset+16..offset+16+len];
        let seq = extract_seq(payload).unwrap_or(0);

        let obj = json!({
            "seq": seq,
            "type": record_name(rt),
            "len": len,
            "crc32": format!("0x{:08x}", crc32),
        });
        println!("{}", obj);

        offset += 16 + len;
        count += 1;
    }
    eprintln!("total: {} records", count);
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::WalDump {
            stream_id,
            wal_dir,
            from_seq,
            json,
        } => wal_dump(stream_id, wal_dir, from_seq, json),
        Commands::Dump { file } => dump_file(file),
    }
}
