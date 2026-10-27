use clap::ArgGroup;
use clap::Args;
use clap::Parser;
use clap::Subcommand;
use rsx_cli::cluster;
use rsx_cli::cluster::Cluster;
use rsx_cli::md;
use rsx_cli::order;
use rsx_cli::order::CancelTarget;
use rsx_cli::order::Gateway;
use rsx_cli::order::OrderInput;
use rsx_cli::order::PriceInput;
use rsx_cli::order::QtyInput;
use rsx_cli::order::Side;
use rsx_cli::order::Sizing;
use rsx_cli::order::Tif;
use rsx_cli::output;
use rsx_cli::output::Format;
use rsx_cli::output::Outcome;
use rsx_cli::plan::DEFAULT_DATABASE_URL;
use rsx_cli::plan::DEFAULT_JWT_SECRET;
use rsx_cli::record::DisplayScale;
use rsx_cli::reset;
use rsx_cli::token;
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
    /// Mint gateway tokens
    Token {
        #[command(subcommand)]
        command: TokenCommand,
    },
    /// Place and cancel orders over the gateway WebSocket
    Order {
        #[command(subcommand)]
        command: OrderCommand,
    },
    /// Start, stop and inspect the exchange processes of a scenario
    Cluster {
        #[command(subcommand)]
        command: ClusterCommand,
    },
}

#[derive(Subcommand)]
enum ClusterCommand {
    /// Print the processes a scenario spawns, with env and ports
    Plan(ClusterPlanArgs),
    /// List the scenarios
    Scenarios {
        /// Output one JSON object (default: text)
        #[arg(long)]
        json: bool,
    },
    /// Start a scenario's processes and wait until they report ready
    Up(ClusterUpArgs),
    /// Stop the processes the cluster started
    Down(ClusterDownArgs),
    /// List the cluster's processes and their state
    Ps(ClusterPsArgs),
    /// Stop one process and start it again
    Restart(ClusterRestartArgs),
    /// Stop the cluster, then wipe the WAL, the replay tips and risk's Postgres state
    Reset(ClusterResetArgs),
}

#[derive(Args)]
struct ClusterRootArgs {
    /// Repository root: binaries, tmp/, log/ and certs/ live under it
    #[arg(short = 'C', long, env = "RSX_ROOT", default_value = ".")]
    root: PathBuf,
    /// Output one JSON object (default: text)
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct ClusterArgs {
    #[command(flatten)]
    root: ClusterRootArgs,
    /// Postgres url with risk's symbol config (the source of the symbols)
    #[arg(long, env = "DATABASE_URL", default_value = DEFAULT_DATABASE_URL)]
    db: String,
    /// HS256 secret the gateway and the sidecars share
    #[arg(
        long,
        env = "RSX_GW_JWT_SECRET",
        default_value = DEFAULT_JWT_SECRET,
        hide_env_values = true
    )]
    secret: String,
    /// Use target/release binaries
    #[arg(short, long)]
    release: bool,
    /// Pin each daemon to a core through its RSX_<X>_CORE_ID
    #[arg(long)]
    pin_cores: bool,
    /// Leave the maker out (it starts with every scenario otherwise)
    #[arg(long)]
    no_maker: bool,
}

impl ClusterArgs {
    fn to_cluster(&self) -> Cluster {
        Cluster {
            root: self.root.root.clone(),
            database_url: self.db.clone(),
            jwt_secret: self.secret.clone(),
            release: self.release,
            pin_cores: self.pin_cores,
            no_maker: self.no_maker,
        }
    }
}

#[derive(Args)]
struct ClusterPlanArgs {
    #[command(flatten)]
    cluster: ClusterArgs,
    /// Scenario to plan
    #[arg(short, long, default_value = "minimal")]
    scenario: String,
}

#[derive(Args)]
struct ClusterUpArgs {
    #[command(flatten)]
    cluster: ClusterArgs,
    /// Scenario to start
    #[arg(short, long, default_value = "minimal")]
    scenario: String,
    /// Start only this process (repeatable), e.g. maker or auth
    #[arg(short, long)]
    only: Vec<String>,
    /// Run cargo build for the daemons first
    #[arg(short, long)]
    build: bool,
    /// Seconds to wait for every process to report ready
    #[arg(short, long, default_value_t = 120)]
    wait_s: u64,
}

#[derive(Args)]
struct ClusterDownArgs {
    #[command(flatten)]
    root: ClusterRootArgs,
    /// Stop only this process (repeatable)
    #[arg(short, long)]
    only: Vec<String>,
}

#[derive(Args)]
struct ClusterPsArgs {
    #[command(flatten)]
    root: ClusterRootArgs,
}

#[derive(Args)]
struct ClusterRestartArgs {
    #[command(flatten)]
    cluster: ClusterArgs,
    /// Process to restart, e.g. risk-0
    name: String,
    /// Scenario to rebuild the process from (default: the last one started)
    #[arg(short, long)]
    scenario: Option<String>,
    /// Seconds to wait for the process to report ready
    #[arg(short, long, default_value_t = 120)]
    wait_s: u64,
}

#[derive(Args)]
struct ClusterResetArgs {
    #[command(flatten)]
    root: ClusterRootArgs,
    /// Postgres url with risk's state (positions, fills, funding, tips)
    #[arg(long, env = "DATABASE_URL", default_value = DEFAULT_DATABASE_URL)]
    db: String,
}

#[derive(Subcommand)]
enum OrderCommand {
    /// Send one order and print how it ended
    Place(OrderPlaceArgs),
    /// Cancel one pending order of a user by client order id or order id
    Cancel(OrderCancelArgs),
    /// Send JSON-lines orders one at a time and print one outcome per order
    Batch(OrderBatchArgs),
}

#[derive(Args)]
struct GatewayArgs {
    /// Gateway WebSocket url
    #[arg(
        short,
        long,
        env = "GATEWAY_URL",
        default_value = "ws://127.0.0.1:8088"
    )]
    gateway: String,
    /// HS256 secret shared with the gateway
    #[arg(long, env = "RSX_GW_JWT_SECRET", hide_env_values = true)]
    secret: Option<String>,
    /// How long to wait for each order's outcome, in milliseconds
    #[arg(short, long, default_value_t = 5000)]
    timeout_ms: u64,
    /// Output JSON (default: text)
    #[arg(long)]
    json: bool,
}

impl GatewayArgs {
    fn to_gateway(&self) -> Gateway {
        Gateway {
            url: self.gateway.clone(),
            secret: self.secret.clone(),
            timeout: Duration::from_millis(self.timeout_ms),
        }
    }
}

#[derive(Args)]
#[command(
    group(ArgGroup::new("price_unit").required(true).args(["price", "px"])),
    group(ArgGroup::new("qty_unit").required(true).args(["qty", "qty_raw", "min_size"]))
)]
struct OrderPlaceArgs {
    /// User the order is for
    #[arg(short, long)]
    user: u32,
    /// Symbol to trade
    #[arg(short, long)]
    symbol: u32,
    #[arg(long, value_enum)]
    side: Side,
    /// Price in human units, converted with the symbol's price decimals
    #[arg(short, long)]
    price: Option<String>,
    /// Price in raw ticks
    #[arg(long)]
    px: Option<i64>,
    /// Quantity in human units, converted with the symbol's qty decimals
    #[arg(short, long)]
    qty: Option<String>,
    /// Quantity in raw lots
    #[arg(long)]
    qty_raw: Option<i64>,
    /// The smallest quantity that meets the symbol's minimum order size at
    /// the price
    #[arg(long)]
    min_size: bool,
    /// Minimum notional in raw price x qty units for --min-size (default: the
    /// symbol's from --db, else 1 USD from its decimals)
    #[arg(long)]
    min_notional: Option<i64>,
    /// Postgres url to read the symbol's minimum notional from
    #[arg(long, env = "DATABASE_URL")]
    db: Option<String>,
    #[arg(long, value_enum, default_value_t = Tif::Gtc)]
    tif: Tif,
    #[arg(long)]
    reduce_only: bool,
    #[arg(long)]
    post_only: bool,
    /// Client order id, 1 to 20 of 0-9 A-Z a-z - _ (default: a fresh one)
    #[arg(long)]
    cid: Option<String>,
    #[command(flatten)]
    gateway: GatewayArgs,
}

#[derive(Args)]
struct OrderCancelArgs {
    /// User the order belongs to
    #[arg(short, long)]
    user: u32,
    /// Client order id (1 to 20 characters) or order id (32 hex characters)
    key: String,
    #[command(flatten)]
    gateway: GatewayArgs,
}

#[derive(Args)]
struct OrderBatchArgs {
    /// JSON-lines file of orders (default: standard input)
    file: Option<PathBuf>,
    /// Minimum notional in raw price x qty units for min_size orders
    #[arg(long)]
    min_notional: Option<i64>,
    /// Postgres url to read each symbol's minimum notional from
    #[arg(long, env = "DATABASE_URL")]
    db: Option<String>,
    #[command(flatten)]
    gateway: GatewayArgs,
}

#[derive(Subcommand)]
enum TokenCommand {
    /// Print a gateway JWT for a user
    Mint(TokenMintArgs),
}

#[derive(Args)]
struct TokenMintArgs {
    /// User the token is for
    #[arg(short, long)]
    user: u32,
    /// Seconds until the token expires
    #[arg(short, long, default_value_t = 3600)]
    ttl: u64,
    /// HS256 secret shared with the gateway
    #[arg(short, long, env = "RSX_GW_JWT_SECRET", hide_env_values = true)]
    secret: Option<String>,
    /// Output one JSON object (default: the bare token)
    #[arg(long)]
    json: bool,
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

fn run_token(command: &TokenCommand) -> Outcome {
    match command {
        TokenCommand::Mint(args) => {
            token::mint(args.user, args.ttl, args.secret.as_deref(), args.json)
        }
    }
}

impl TokenCommand {
    fn is_json(&self) -> bool {
        match self {
            TokenCommand::Mint(args) => args.json,
        }
    }
}

fn run_order(command: &OrderCommand) -> Outcome {
    match command {
        OrderCommand::Place(args) => {
            let price = match (&args.price, args.px) {
                (Some(price), _) => PriceInput::Human(price.clone()),
                (None, Some(px)) => PriceInput::Raw(px),
                (None, None) => return Err(output::Failure::misuse("a price is required")),
            };
            let qty = match (&args.qty, args.qty_raw) {
                (Some(qty), _) => QtyInput::Human(qty.clone()),
                (None, Some(qty_raw)) => QtyInput::Raw(qty_raw),
                (None, None) => QtyInput::Minimum,
            };
            let input = OrderInput {
                user_id: args.user,
                symbol_id: args.symbol,
                side: args.side,
                tif: args.tif,
                price,
                qty,
                reduce_only: args.reduce_only,
                post_only: args.post_only,
                cid: args.cid.clone(),
            };
            let sizing = Sizing {
                min_notional: args.min_notional,
                database_url: args.db.clone(),
            };
            order::place(
                &args.gateway.to_gateway(),
                &input,
                &sizing,
                args.gateway.json,
            )
        }
        OrderCommand::Cancel(args) => {
            let target: CancelTarget = order::cancel_target(&args.key)?;
            order::cancel(
                &args.gateway.to_gateway(),
                args.user,
                &target,
                args.gateway.json,
            )
        }
        OrderCommand::Batch(args) => {
            let inputs = match &args.file {
                Some(path) => {
                    let file = std::fs::File::open(path).map_err(|error| {
                        output::Failure::misuse(format!("cannot open {}: {error}", path.display()))
                    })?;
                    order::read_batch(file)?
                }
                None => order::read_batch(std::io::stdin())?,
            };
            let sizing = Sizing {
                min_notional: args.min_notional,
                database_url: args.db.clone(),
            };
            order::batch(
                &args.gateway.to_gateway(),
                &inputs,
                &sizing,
                args.gateway.json,
            )
        }
    }
}

impl OrderCommand {
    fn is_json(&self) -> bool {
        match self {
            OrderCommand::Place(args) => args.gateway.json,
            OrderCommand::Cancel(args) => args.gateway.json,
            OrderCommand::Batch(args) => args.gateway.json,
        }
    }
}

fn run_cluster(command: &ClusterCommand) -> Outcome {
    match command {
        ClusterCommand::Plan(args) => args
            .cluster
            .to_cluster()
            .plan(&args.scenario, args.cluster.root.json),
        ClusterCommand::Scenarios { json } => cluster::scenarios(*json),
        ClusterCommand::Up(args) => args.cluster.to_cluster().up(
            &args.scenario,
            &args.only,
            args.build,
            Duration::from_secs(args.wait_s),
            args.cluster.root.json,
        ),
        ClusterCommand::Down(args) => cluster::down(&args.root.root, &args.only, args.root.json),
        ClusterCommand::Ps(args) => cluster::ps(&args.root.root, args.root.json),
        ClusterCommand::Restart(args) => args.cluster.to_cluster().restart(
            &args.name,
            args.scenario.as_deref(),
            Duration::from_secs(args.wait_s),
            args.cluster.root.json,
        ),
        ClusterCommand::Reset(args) => reset::reset(&args.root.root, &args.db, args.root.json),
    }
}

impl ClusterCommand {
    fn is_json(&self) -> bool {
        match self {
            ClusterCommand::Plan(args) => args.cluster.root.json,
            ClusterCommand::Scenarios { json } => *json,
            ClusterCommand::Up(args) => args.cluster.root.json,
            ClusterCommand::Down(args) => args.root.json,
            ClusterCommand::Ps(args) => args.root.json,
            ClusterCommand::Restart(args) => args.cluster.root.json,
            ClusterCommand::Reset(args) => args.root.json,
        }
    }
}

fn main() -> ExitCode {
    let (outcome, is_json) = match Cli::parse().command {
        Command::Wal { command } => (run_wal(&command), command.is_json()),
        Command::Verify(args) => (run_verify(&args), args.json),
        Command::Md { command } => (run_md(&command), command.is_json()),
        Command::Token { command } => (run_token(&command), command.is_json()),
        Command::Order { command } => (run_order(&command), command.is_json()),
        Command::Cluster { command } => (run_cluster(&command), command.is_json()),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => output::report(&failure, is_json),
    }
}
