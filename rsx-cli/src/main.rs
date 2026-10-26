use clap::Args;
use clap::Parser;
use clap::Subcommand;
use rsx_cli::output;
use rsx_cli::output::Format;
use rsx_cli::output::Outcome;
use rsx_cli::record::DisplayScale;
use rsx_cli::wal;
use rsx_cli::wal::Filters;
use rsx_cli::wal::Stream;
use std::path::PathBuf;
use std::process::ExitCode;

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
}

#[derive(Subcommand)]
enum WalCommand {
    /// List the streams of a WAL directory
    Ls(DirArgs),
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
            WalCommand::Ls(args) | WalCommand::Verify(args) => args.json,
            WalCommand::Dump(args) => args.json,
            WalCommand::Stats(args) => args.json,
            WalCommand::Types { json } => *json,
            WalCommand::File { .. } => false,
        }
    }
}

fn run_wal(command: &WalCommand) -> Outcome {
    match command {
        WalCommand::Ls(args) => wal::ls(&args.wal_dir, &args.stream, &args.to_format()),
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

fn main() -> ExitCode {
    let Command::Wal { command } = Cli::parse().command;
    match run_wal(&command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => output::report(&failure, command.is_json()),
    }
}
