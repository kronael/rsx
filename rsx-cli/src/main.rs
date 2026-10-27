use clap::Args;
use clap::Parser;
use clap::Subcommand;
use rsx_cli::md;
use rsx_cli::output;
use rsx_cli::output::Format;
use rsx_cli::output::Outcome;
use rsx_cli::record::DisplayScale;
use rsx_cli::verify;
use rsx_cli::verify::Options;
use rsx_cli::wal;
use rsx_cli::wal::Filters;
use rsx_cli::wal::Stream;
use rsx_marketdata::subscription::CHANNEL_BBO;
use rsx_marketdata::subscription::CHANNEL_DEPTH;
use rsx_marketdata::subscription::CHANNEL_TRADES;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "rsx-cli", version, about = "RSX CLI tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect WAL files
    Wal {
        #[command(subcommand)]
        command: WalCommand,
    },
    /// Check the exchange invariants over WAL directories and Postgres
    Verify(VerifyArgs),
    /// Read the public market-data feed
    Md {
        #[command(subcommand)]
        command: MdCommand,
    },
}

#[derive(Subcommand)]
enum MdCommand {
    /// Print the L2 book marketdata serves for a symbol
    Snapshot(MdSnapshotArgs),
    /// Print every frame for the symbols until Ctrl-C
    Follow(MdFollowArgs),
}

#[derive(Args)]
struct MdSnapshotArgs {
    /// Symbol to read
    symbol_id: u32,
    /// Marketdata WebSocket url
    #[arg(
        short,
        long,
        env = "MARKETDATA_WS",
        default_value = "ws://127.0.0.1:8180"
    )]
    url: String,
    /// How long to wait for the snapshot, in milliseconds
    #[arg(short, long, default_value_t = 2000)]
    timeout_ms: u64,
    /// Output one JSON object (default: text)
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct MdFollowArgs {
    /// Symbols to subscribe to
    #[arg(required = true)]
    symbol_ids: Vec<u32>,
    /// Marketdata WebSocket url
    #[arg(
        short,
        long,
        env = "MARKETDATA_WS",
        default_value = "ws://127.0.0.1:8180"
    )]
    url: String,
    /// Channel bitmask: 1 BBO, 2 depth, 4 trades
    #[arg(
        short,
        long,
        default_value_t = CHANNEL_BBO | CHANNEL_DEPTH | CHANNEL_TRADES,
        value_parser = clap::value_parser!(u32).range(1..=7)
    )]
    channels: u32,
    /// Stop after this many frames
    #[arg(short = 'n', long)]
    count: Option<u64>,
    /// Output JSON lines (default: text)
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct VerifyArgs {
    /// WAL directories, each holding numeric stream directories
    #[arg(required = true)]
    wal_dirs: Vec<PathBuf>,
    /// Only this stream ID (repeatable); default: every stream directory
    #[arg(short, long)]
    stream: Vec<u32>,
    /// Postgres url for the checks that read risk's tables
    #[arg(long, env = "DATABASE_URL")]
    db: Option<String>,
    /// Output one JSON object (default: text)
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum WalCommand {
    /// List the streams of a WAL directory
    Ls(LsArgs),
    /// Check the CRC and sequence continuity of every record
    Verify(DirArgs),
    /// Dump the records of a stream
    Dump(DumpArgs),
    /// Print per-type record counts of a stream
    Stats(StatsArgs),
    /// List the record type names `--type` accepts
    Types {
        /// Output one JSON object (default: text)
        #[arg(long)]
        json: bool,
    },
    /// Dump a single WAL segment file as JSON lines
    File {
        /// WAL segment file path
        file: PathBuf,
    },
}

#[derive(Args)]
struct DirArgs {
    /// WAL directory path
    wal_dir: PathBuf,
    /// Only this stream ID (repeatable); default: every stream directory
    #[arg(short, long)]
    stream: Vec<u32>,
    /// Output one JSON object (default: text)
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct LsArgs {
    #[command(flatten)]
    dir: DirArgs,
    /// Do not read records: first_seq, last_seq and last_ts_ns are null
    #[arg(long)]
    no_seq: bool,
}

#[derive(Args)]
struct StreamArgs {
    /// Stream ID to read
    stream_id: u32,
    /// WAL directory path
    wal_dir: PathBuf,
    /// First sequence number to emit
    #[arg(default_value = "0")]
    from_seq: u64,
}

#[derive(Args)]
struct FilterArgs {
    /// Only emit records of this type (repeatable, OR logic)
    #[arg(long = "type", value_name = "TYPE")]
    record_types: Vec<String>,
    /// Filter by symbol_id
    #[arg(long)]
    symbol: Option<u32>,
    /// Filter by user_id
    #[arg(long)]
    user: Option<u32>,
    /// Skip records with ts_ns < value
    #[arg(long)]
    from_ts: Option<u64>,
    /// Skip records with ts_ns > value
    #[arg(long)]
    to_ts: Option<u64>,
}

#[derive(Args)]
struct DumpArgs {
    #[command(flatten)]
    stream: StreamArgs,
    #[command(flatten)]
    filters: FilterArgs,
    /// Output as JSON lines (default: text)
    #[arg(long)]
    json: bool,
    /// After the last record keep polling for new ones until Ctrl-C
    #[arg(long)]
    follow: bool,
    /// Divide raw i64 prices by this for text display
    #[arg(long)]
    tick_size: Option<f64>,
    /// Divide raw i64 quantities by this for text display
    #[arg(long)]
    lot_size: Option<f64>,
}

#[derive(Args)]
struct StatsArgs {
    #[command(flatten)]
    stream: StreamArgs,
    #[command(flatten)]
    filters: FilterArgs,
    /// Output one JSON object (default: text)
    #[arg(long)]
    json: bool,
}

impl DirArgs {
    fn to_format(&self) -> Format {
        if self.json {
            Format::Json
        } else {
            Format::Text(DisplayScale::new(None, None))
        }
    }
}

impl StreamArgs {
    fn to_stream(&self) -> Stream {
        Stream {
            stream_id: self.stream_id,
            wal_dir: self.wal_dir.clone(),
            from_seq: self.from_seq,
        }
    }
}

impl FilterArgs {
    fn to_filters(&self) -> Result<Filters, output::Failure> {
        Filters::new(
            &self.record_types,
            self.symbol,
            self.user,
            self.from_ts,
            self.to_ts,
        )
    }
}

impl WalCommand {
    fn is_json(&self) -> bool {
        match self {
            WalCommand::Ls(args) => args.dir.json,
            WalCommand::Verify(args) => args.json,
            WalCommand::Dump(args) => args.json,
            WalCommand::Stats(args) => args.json,
            WalCommand::Types { json } => *json,
            WalCommand::File { .. } => false,
        }
    }
}

fn run_wal(command: &WalCommand) -> Outcome {
    match command {
        WalCommand::Ls(args) => wal::ls(
            &args.dir.wal_dir,
            &args.dir.stream,
            !args.no_seq,
            &args.dir.to_format(),
        ),
        WalCommand::Verify(args) => wal::verify(&args.wal_dir, &args.stream, &args.to_format()),
        WalCommand::Dump(args) => {
            let filters = args.filters.to_filters()?;
            let stream = args.stream.to_stream();
            let format = if args.json {
                Format::Json
            } else {
                Format::Text(DisplayScale::new(args.tick_size, args.lot_size))
            };
            if args.follow {
                wal::follow(&stream, &filters, &format)
            } else {
                wal::dump(&stream, &filters, &format)
            }
        }
        WalCommand::Stats(args) => {
            let filters = args.filters.to_filters()?;
            let format = if args.json {
                Format::Json
            } else {
                Format::Text(DisplayScale::new(None, None))
            };
            wal::stats(&args.stream.to_stream(), &filters, &format)
        }
        WalCommand::Types { json } => {
            let format = if *json {
                Format::Json
            } else {
                Format::Text(DisplayScale::new(None, None))
            };
            wal::types(&format)
        }
        WalCommand::File { file } => wal::dump_file(file),
    }
}

fn run_verify(args: &VerifyArgs) -> Outcome {
    let options = Options {
        wal_dirs: args.wal_dirs.clone(),
        only: args.stream.clone(),
        database_url: args.db.clone(),
    };
    let format = if args.json {
        Format::Json
    } else {
        Format::Text(DisplayScale::new(None, None))
    };
    verify::verify(&options, &format)
}

fn run_md(command: &MdCommand) -> Outcome {
    match command {
        MdCommand::Snapshot(args) => md::snapshot(
            &args.url,
            args.symbol_id,
            Duration::from_millis(args.timeout_ms),
            args.json,
        ),
        MdCommand::Follow(args) => md::follow(
            &args.url,
            &args.symbol_ids,
            args.channels,
            args.count,
            args.json,
        ),
    }
}

impl MdCommand {
    fn is_json(&self) -> bool {
        match self {
            MdCommand::Snapshot(args) => args.json,
            MdCommand::Follow(args) => args.json,
        }
    }
}

fn main() -> ExitCode {
    let (outcome, is_json) = match Cli::parse().command {
        Command::Wal { command } => (run_wal(&command), command.is_json()),
        Command::Verify(args) => (run_verify(&args), args.json),
        Command::Md { command } => (run_md(&command), command.is_json()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => output::report(&failure, is_json),
    }
}
