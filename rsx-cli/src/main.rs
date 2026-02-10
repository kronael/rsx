use clap::Parser;
use clap::Subcommand;
use rsx_dxs::records::*;
use rsx_dxs::wal::extract_seq;
use rsx_dxs::wal::WalReader;
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
) {
    let mut reader = WalReader::open_from_seq(
        stream_id, from_seq, &wal_dir,
    )
    // SAFETY: fail-fast at startup
    .expect("failed to open wal");

    let mut count: u64 = 0;
    while let Ok(Some(raw)) = reader.next() {
        let rt = raw.header.record_type;
        let len = raw.header.len;
        let seq =
            extract_seq(&raw.payload).unwrap_or(0);
        println!(
            "seq={:<8} type={:<18} len={:<4} \
             crc=0x{:08x}",
            seq,
            record_name(rt),
            len,
            raw.header.crc32,
        );
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
        } => wal_dump(stream_id, wal_dir, from_seq),
    }
}
