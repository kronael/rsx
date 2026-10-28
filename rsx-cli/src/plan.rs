use crate::output::Failure;
use crate::scenario::Scenario;
use crate::scenario::SYMBOL_ORDER;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(test)]
#[path = "plan_test.rs"]
mod plan_test;

pub const DEFAULT_DATABASE_URL: &str = "postgres://rsx:rsx@127.0.0.1:5432/rsx";
pub const DEFAULT_JWT_SECRET: &str = "rsx-dev-secret-not-for-prod-padpad";
pub const MAKER_CONFIG_FILE: &str = "./tmp/maker-config.json";
pub const MAKER_NAME: &str = "maker";
pub const AUTH_NAME: &str = "auth";
pub const GATEWAY_WS_PORT: u16 = 8088;
pub const MARKETDATA_WS_PORT: u16 = 8180;

const BOOK_CAPACITY: u32 = 65_536;
const RISK_SHARDS: u32 = 1;

const ME_CAST: u32 = 9100;
const RISK_CAST: u32 = 9200;
const GW_CAST: u32 = 9300;
const MARK_CAST: u32 = 9400;
const MD_CAST: u32 = 9500;
const RISK_MARK_CAST: u32 = 9600;
const ME_REPLICATION: u32 = 9700;
const RISK_ME_RECV: u32 = 28_300;
const RISK_REPLICATION: u32 = 10_400;

// Sender sockets. A receiver sends its NAKs to the sender's bind address,
// so each sender binds one fixed port and its peer's receiver names it.
const GW_SEND: u32 = 11_000;
const RISK_ME_SEND: u32 = 11_100;
const RISK_GW_SEND: u32 = 11_200;
const ME_RISK_SEND: u32 = 11_300;
const ME_MD_SEND: u32 = 11_400;
const MARK_SEND: u32 = 11_500;

const ME_HEALTH: u32 = 9800;
const RISK_HEALTH: u32 = 9900;
const GW_HEALTH: u32 = 10_000;
const MARK_HEALTH: u32 = 10_100;
const MD_HEALTH: u32 = 10_200;
const RECORDER_HEALTH: u32 = 10_300;

const CORE_GW: u32 = 1;
const CORE_RISK: u32 = 2;
const CORE_ME_0: u32 = 3;
const CORE_MD: u32 = 4;

/// One symbol as the cluster needs it, read from risk's Postgres.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogSymbol {
    pub symbol_id: u32,
    pub name: String,
    pub tick_size: i64,
    pub lot_size: i64,
    pub price_decimals: u32,
    pub qty_decimals: u32,
}

/// The maker's quoting knobs; the file at `MAKER_CONFIG_FILE` overrides them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MakerTuning {
    pub spread_bps: i64,
    pub qty: i64,
    pub symbol_id: i64,
    pub refresh_ms: i64,
    pub levels: i64,
    pub jitter_bps: i64,
}

impl Default for MakerTuning {
    fn default() -> Self {
        Self {
            spread_bps: 20,
            qty: 10,
            symbol_id: 10,
            refresh_ms: 500,
            levels: 5,
            jitter_bps: 8,
        }
    }
}

impl MakerTuning {
    /// The defaults with every integer key present in the JSON `text`
    /// replaced; a malformed file leaves the defaults.
    pub fn from_json(text: &str) -> Self {
        let mut tuning = Self::default();
        let Ok(Value::Object(file)) = serde_json::from_str::<Value>(text) else {
            return tuning;
        };
        let read = |key: &str, slot: &mut i64| {
            if let Some(value) = file.get(key).and_then(Value::as_i64) {
                *slot = value;
            }
        };
        read("spread_bps", &mut tuning.spread_bps);
        read("qty", &mut tuning.qty);
        read("symbol_id", &mut tuning.symbol_id);
        read("refresh_ms", &mut tuning.refresh_ms);
        read("levels", &mut tuning.levels);
        read("jitter_bps", &mut tuning.jitter_bps);
        tuning
    }
}

/// The settings of the rsx-auth sidecar that are not secrets; the GitHub
/// client id and secret reach the process through the caller's environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthSetup {
    pub listen: String,
    pub redirect_uri: String,
    pub trade_ui_url: String,
    pub starter_collateral: String,
    pub jwt_ttl_s: String,
}

impl Default for AuthSetup {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8082".to_string(),
            redirect_uri: "http://localhost:49171/oauth/github/callback".to_string(),
            trade_ui_url: String::new(),
            starter_collateral: "0".to_string(),
            jwt_ttl_s: (7 * 24 * 3600).to_string(),
        }
    }
}

/// What `build` needs besides the scenario and the symbols. `maker` and
/// `auth` add their sidecar to the plan when set.
#[derive(Clone, Debug)]
pub struct Options {
    pub database_url: String,
    pub release: bool,
    pub pin_cores: bool,
    pub jwt_secret: String,
    pub maker: Option<MakerTuning>,
    pub auth: Option<AuthSetup>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            database_url: DEFAULT_DATABASE_URL.to_string(),
            release: false,
            pin_cores: false,
            jwt_secret: DEFAULT_JWT_SECRET.to_string(),
            maker: None,
            auth: None,
        }
    }
}

/// One process to spawn: `binary` and `cwd` are relative to the cluster root,
/// `env` is added to the caller's environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub name: String,
    pub binary: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    /// Env values naming the addresses the process binds: `host:port`, or
    /// a comma list of `host:port` and `symbol_id=host:port`.
    pub bound: Vec<String>,
}

/// The ordered processes of a scenario.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub scenario: String,
    pub processes: Vec<Process>,
}

impl Process {
    /// The ports the process binds, ascending. A destination it only sends
    /// to is not among them.
    pub fn ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self
            .bound
            .iter()
            .flat_map(|value| value.split(','))
            .filter_map(|address| address.rsplit_once(':'))
            .filter_map(|(_, port)| port.parse().ok())
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports
    }

    /// The `host:port` of the health server, `None` for sidecars without one.
    pub fn health_addr(&self) -> Option<&str> {
        self.env
            .iter()
            .find(|(key, _)| key.ends_with("_HEALTH_ADDR"))
            .map(|(_, value)| value.as_str())
    }

    /// A warm risk replica: live but never ready until it is promoted.
    pub fn is_replica(&self) -> bool {
        self.name.contains("-replica-")
    }

    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "binary": self.binary,
            "args": self.args,
            "cwd": self.cwd,
            "env": self.env,
            "ports": self.ports(),
        })
    }
}

impl Plan {
    pub fn to_json(&self) -> Value {
        json!({
            "scenario": self.scenario,
            "processes": self.processes.iter().map(Process::to_json).collect::<Vec<_>>(),
        })
    }

    pub fn process(&self, name: &str) -> Option<&Process> {
        self.processes.iter().find(|process| process.name == name)
    }

    /// The health addresses of the risk nodes that share `process`'s shard,
    /// itself included; empty when `process` is not a risk node.
    pub fn shard_health_addrs(&self, process: &Process) -> Vec<&str> {
        let Some(shard) = process.env.get("RSX_RISK_SHARD_ID") else {
            return Vec::new();
        };
        self.processes
            .iter()
            .filter(|peer| peer.env.get("RSX_RISK_SHARD_ID") == Some(shard))
            .filter_map(Process::health_addr)
            .collect()
    }

    /// The highest core id any process pins to, `None` when none pins.
    pub fn max_core_id(&self) -> Option<u32> {
        self.processes
            .iter()
            .flat_map(|process| process.env.iter())
            .filter(|(key, _)| key.ends_with("_CORE_ID"))
            .filter_map(|(_, value)| value.parse().ok())
            .max()
    }
}

/// The first `scenario.symbols` names of `SYMBOL_ORDER` found in `catalog`,
/// or a `Runtime` failure naming the missing ones.
pub fn select_symbols(
    scenario: &Scenario,
    catalog: &[CatalogSymbol],
) -> Result<Vec<CatalogSymbol>, Failure> {
    let wanted: Vec<&str> = SYMBOL_ORDER
        .iter()
        .copied()
        .take(scenario.symbols)
        .collect();
    let mut selected = Vec::new();
    let mut missing = Vec::new();
    for name in &wanted {
        match catalog.iter().find(|symbol| symbol.name == **name) {
            Some(symbol) => selected.push(symbol.clone()),
            None => missing.push(*name),
        }
    }
    if !missing.is_empty() {
        return Err(Failure::runtime(format!(
            "scenario {} needs {} but postgres has no active config for {}",
            scenario.name,
            wanted.join(", "),
            missing.join(", ")
        )));
    }
    Ok(selected)
}

fn text(value: impl ToString) -> String {
    value.to_string()
}

fn addr(base: u32, offset: u32) -> String {
    format!("127.0.0.1:{}", base + offset)
}

/// The env keys whose values `binary` binds; every other address in its env
/// is a destination. Marketdata also binds one cast port per engine, at the
/// engine's cast port plus 400.
fn bound_keys(binary: &str) -> &'static [&'static str] {
    match binary {
        "rsx-matching" => &[
            "RSX_ME_CAST_ADDR",
            "RSX_ME_RISK_SEND_ADDR",
            "RSX_ME_MD_SEND_ADDR",
            "RSX_ME_REPLICATION_BIND_ADDR",
            "RSX_ME_HEALTH_ADDR",
        ],
        "rsx-mark" => &[
            "RSX_MARK_LISTEN_ADDR",
            "RSX_MARK_SEND_ADDR",
            "RSX_MARK_HEALTH_ADDR",
        ],
        "rsx-risk" => &[
            "RSX_RISK_CAST_ADDR",
            "RSX_RISK_MARK_CAST_ADDR",
            "RSX_RISK_ME_RECV_ADDRS",
            "RSX_RISK_ME_SEND_ADDRS",
            "RSX_RISK_GW_SEND_ADDR",
            "RSX_RISK_REPLICATION_BIND_ADDR",
            "RSX_RISK_HEALTH_ADDR",
        ],
        "rsx-gateway" => &[
            "RSX_GW_LISTEN",
            "RSX_GW_CAST_ADDR",
            "RSX_GW_SEND_ADDR",
            "RSX_GW_HEALTH_ADDR",
        ],
        "rsx-marketdata" => &["RSX_MD_LISTEN", "RSX_MD_HEALTH_ADDR"],
        "rsx-recorder" => &["RSX_RECORDER_HEALTH_ADDR"],
        _ => &[],
    }
}

fn bound_by(env: &BTreeMap<String, String>, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .filter_map(|key| env.get(*key).cloned())
        .collect()
}

fn env_of<const N: usize>(pairs: [(&str, String); N]) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn join_symbols(symbols: &[CatalogSymbol], each: impl Fn(&CatalogSymbol) -> String) -> String {
    symbols.iter().map(each).collect::<Vec<_>>().join(",")
}

struct Layout<'a> {
    symbols: &'a [CatalogSymbol],
    options: &'a Options,
    target: &'a str,
    me_cast_addrs: String,
    risk_me_recv_addrs: String,
    me_replication_addrs: String,
    max_symbols: u32,
}

impl Layout<'_> {
    fn binary(&self, name: &str) -> String {
        format!("{}/{name}", self.target)
    }

    fn pin(&self, env: &mut BTreeMap<String, String>, key: &str, core: u32) {
        if self.options.pin_cores {
            env.insert(key.to_string(), core.to_string());
        }
    }

    fn process(&self, name: String, binary: &str, env: BTreeMap<String, String>) -> Process {
        Process {
            name,
            binary: self.binary(binary),
            args: Vec::new(),
            cwd: ".".to_string(),
            bound: bound_by(&env, bound_keys(binary)),
            env,
        }
    }

    fn matching(&self, symbol: &CatalogSymbol) -> Process {
        let id = symbol.symbol_id;
        let mut env = env_of([
            ("RSX_ME_SYMBOL_ID", text(id)),
            ("RSX_ME_PRICE_DECIMALS", text(symbol.price_decimals)),
            ("RSX_ME_QTY_DECIMALS", text(symbol.qty_decimals)),
            ("RSX_ME_TICK_SIZE", text(symbol.tick_size)),
            ("RSX_ME_LOT_SIZE", text(symbol.lot_size)),
            ("RSX_ME_BOOK_CAPACITY", text(BOOK_CAPACITY)),
            (
                "RSX_ME_WAL_DIR",
                format!("./tmp/wal/{}", symbol.name.to_lowercase()),
            ),
            ("RSX_ME_CAST_ADDR", addr(ME_CAST, id)),
            ("RSX_RISK_CAST_ADDR", addr(RISK_CAST, 0)),
            ("RSX_RISK_ME_RECV_ADDR", addr(RISK_ME_RECV, id)),
            ("RSX_RISK_ME_SEND_ADDR", addr(RISK_ME_SEND, id)),
            ("RSX_ME_RISK_SEND_ADDR", addr(ME_RISK_SEND, id)),
            ("RSX_MD_CAST_ADDR", addr(MD_CAST, id)),
            ("RSX_ME_MD_SEND_ADDR", addr(ME_MD_SEND, id)),
            ("RSX_ME_REPLICATION_BIND_ADDR", addr(ME_REPLICATION, id)),
            ("RSX_ME_DATABASE_URL", self.options.database_url.clone()),
            ("RSX_ME_HEALTH_ADDR", addr(ME_HEALTH, id)),
            ("RUST_LOG", text("info")),
        ]);
        let offset = self
            .symbols
            .iter()
            .filter(|other| other.symbol_id < id)
            .count() as u32;
        self.pin(&mut env, "RSX_ME_CORE_ID", CORE_ME_0 + offset);
        self.process(
            format!("me-{}", symbol.name.to_lowercase()),
            "rsx-matching",
            env,
        )
    }

    fn mark(&self) -> Process {
        let mut pairs = Vec::new();
        for symbol in self.symbols {
            let id = symbol.symbol_id;
            pairs.push(format!("{}={id}", symbol.name));
            pairs.push(format!("{}USDT={id}", symbol.name));
            pairs.push(format!("{}-USD={id}", symbol.name));
        }
        let streams = self
            .symbols
            .iter()
            .map(|symbol| format!("{}usdt@trade", symbol.name.to_lowercase()))
            .collect::<Vec<_>>()
            .join("/");
        let env = env_of([
            ("RSX_MARK_LISTEN_ADDR", addr(MARK_CAST, 0)),
            ("RSX_MARK_SEND_ADDR", addr(MARK_SEND, 0)),
            ("RSX_MARK_WAL_DIR", text("./tmp/wal/mark")),
            ("RSX_MARK_STREAM_ID", text("100")),
            ("RSX_MARK_SYMBOL_MAP", pairs.join(",")),
            ("RSX_RISK_MARK_CAST_ADDR", addr(RISK_MARK_CAST, 0)),
            ("RSX_MARK_SOURCE_BINANCE_ENABLED", text("1")),
            (
                "RSX_MARK_SOURCE_BINANCE_WS_URL",
                format!("wss://stream.binance.com:9443/stream?streams={streams}"),
            ),
            ("RSX_MARK_SOURCE_COINBASE_ENABLED", text("0")),
            ("RSX_MARK_HEALTH_ADDR", addr(MARK_HEALTH, 0)),
            ("RUST_LOG", text("info")),
        ]);
        self.process(text("mark"), "rsx-mark", env)
    }

    fn risk_env(&self, shard: u32) -> BTreeMap<String, String> {
        let mut env = env_of([
            ("RSX_RISK_SHARD_ID", text(shard)),
            ("RSX_RISK_SHARD_COUNT", text(RISK_SHARDS)),
            ("RSX_RISK_MAX_SYMBOLS", text(self.max_symbols)),
            ("RSX_RISK_CAST_ADDR", addr(RISK_CAST, shard)),
            ("RSX_GW_CAST_ADDR", addr(GW_CAST, 0)),
            ("RSX_GW_SEND_ADDR", addr(GW_SEND, 0)),
            ("RSX_RISK_GW_SEND_ADDR", addr(RISK_GW_SEND, shard)),
            ("RSX_ME_CAST_ADDRS", self.me_cast_addrs.clone()),
            ("RSX_RISK_ME_RECV_ADDRS", self.risk_me_recv_addrs.clone()),
            (
                "RSX_RISK_ME_SEND_ADDRS",
                join_symbols(self.symbols, |symbol| {
                    format!(
                        "{}={}",
                        symbol.symbol_id,
                        addr(RISK_ME_SEND, symbol.symbol_id)
                    )
                }),
            ),
            (
                "RSX_ME_RISK_SEND_ADDRS",
                join_symbols(self.symbols, |symbol| {
                    format!(
                        "{}={}",
                        symbol.symbol_id,
                        addr(ME_RISK_SEND, symbol.symbol_id)
                    )
                }),
            ),
            (
                "RSX_ME_REPLICATION_ADDRS",
                self.me_replication_addrs.clone(),
            ),
            ("RSX_RISK_WAL_DIR", text("./tmp/wal")),
            (
                "RSX_RISK_REPLICATION_BIND_ADDR",
                addr(RISK_REPLICATION, shard),
            ),
            ("RSX_RISK_MARK_CAST_ADDR", addr(RISK_MARK_CAST, 0)),
            ("RSX_MARK_CAST_ADDR", addr(MARK_SEND, 0)),
            ("RSX_RISK_HEALTH_ADDR", addr(RISK_HEALTH, shard)),
            ("DATABASE_URL", self.options.database_url.clone()),
            ("RUST_LOG", text("info")),
        ]);
        self.pin(&mut env, "RSX_RISK_CORE_ID", CORE_RISK + shard);
        env
    }

    fn risk_primary(&self, shard: u32) -> Process {
        self.process(format!("risk-{shard}"), "rsx-risk", self.risk_env(shard))
    }

    fn risk_replica(&self, shard: u32, replica: u32) -> Process {
        let mut env = self.risk_env(shard);
        env.insert(
            "RSX_RISK_HEALTH_ADDR".to_string(),
            addr(RISK_HEALTH + 50, shard * 2 + replica),
        );
        self.process(format!("risk-{shard}-replica-{replica}"), "rsx-risk", env)
    }

    fn gateway(&self) -> Process {
        let mut env = env_of([
            ("RSX_GW_LISTEN", format!("0.0.0.0:{GATEWAY_WS_PORT}")),
            ("RSX_GW_CAST_ADDR", addr(GW_CAST, 0)),
            ("RSX_GW_SEND_ADDR", addr(GW_SEND, 0)),
            ("RSX_RISK_CAST_ADDR", addr(RISK_CAST, 0)),
            ("RSX_RISK_GW_SEND_ADDR", addr(RISK_GW_SEND, 0)),
            ("RSX_RISK_REPLICATION_ADDR", addr(RISK_REPLICATION, 0)),
            ("RSX_GW_WAL_DIR", text("./tmp/wal")),
            ("RSX_GW_JWT_SECRET", self.options.jwt_secret.clone()),
            ("RSX_GW_RL_USER", text("5000")),
            ("RSX_GW_RL_IP", text("10000")),
            ("RSX_GW_RL_INSTANCE", text("100000")),
            ("RSX_GW_DATABASE_URL", self.options.database_url.clone()),
            ("RSX_GW_HEALTH_ADDR", addr(GW_HEALTH, 0)),
            ("RUST_LOG", text("info")),
        ]);
        self.pin(&mut env, "RSX_GW_CORE_ID", CORE_GW);
        self.process(text("gw-0"), "rsx-gateway", env)
    }

    fn marketdata(&self, first: u32) -> Process {
        let mut env = env_of([
            ("RSX_MD_LISTEN", format!("0.0.0.0:{MARKETDATA_WS_PORT}")),
            ("RSX_ME_CAST_ADDRS", self.me_cast_addrs.clone()),
            (
                "RSX_ME_MD_SEND_ADDRS",
                join_symbols(self.symbols, |symbol| addr(ME_MD_SEND, symbol.symbol_id)),
            ),
            ("RSX_MD_STREAM_ID", text(first)),
            ("RSX_MD_REPLAY_ADDR", addr(ME_REPLICATION, first)),
            ("RSX_MD_HEALTH_ADDR", addr(MD_HEALTH, 0)),
            ("RUST_LOG", text("info")),
        ]);
        self.pin(&mut env, "RSX_MD_CORE_ID", CORE_MD);
        let mut process = self.process(text("marketdata"), "rsx-marketdata", env);
        process.bound.push(join_symbols(self.symbols, |symbol| {
            addr(MD_CAST, symbol.symbol_id)
        }));
        process
    }

    fn recorder(&self, first: u32) -> Process {
        let env = env_of([
            ("RSX_RECORDER_STREAM_ID", text(first)),
            ("RSX_RECORDER_PRODUCER_ADDR", addr(ME_REPLICATION, first)),
            ("RSX_RECORDER_ARCHIVE_DIR", text("./tmp/wal/archive")),
            (
                "RSX_RECORDER_TIP_FILE",
                format!("./tmp/recorder-tip-{first}"),
            ),
            ("RSX_RECORDER_HEALTH_ADDR", addr(RECORDER_HEALTH, 0)),
            ("RUST_LOG", text("info")),
        ]);
        self.process(text("recorder"), "rsx-recorder", env)
    }

    fn maker(&self, tuning: &MakerTuning) -> Process {
        let env = env_of([
            ("GATEWAY_URL", format!("ws://127.0.0.1:{GATEWAY_WS_PORT}")),
            (
                "MARKETDATA_WS",
                format!("ws://127.0.0.1:{MARKETDATA_WS_PORT}"),
            ),
            ("RSX_GW_JWT_SECRET", self.options.jwt_secret.clone()),
            ("RSX_MAKER_CONFIG_FILE", text(MAKER_CONFIG_FILE)),
            ("RSX_MAKER_SPREAD_BPS", text(tuning.spread_bps)),
            ("RSX_MAKER_QTY", text(tuning.qty)),
            ("RSX_MAKER_SYMBOL", text(tuning.symbol_id)),
            (
                "RSX_MAKER_SYMBOLS",
                join_symbols(self.symbols, |symbol| text(symbol.symbol_id)),
            ),
            ("RSX_MAKER_REFRESH_MS", text(tuning.refresh_ms)),
            ("RSX_MAKER_LEVELS", text(tuning.levels)),
            ("RSX_MAKER_JITTER_BPS", text(tuning.jitter_bps)),
        ]);
        Process {
            name: text(MAKER_NAME),
            binary: text("./rsx-maker/rsx-maker"),
            args: Vec::new(),
            cwd: text("."),
            bound: Vec::new(),
            env,
        }
    }

    fn auth(&self, setup: &AuthSetup) -> Process {
        let env = env_of([
            ("RSX_AUTH_LISTEN", setup.listen.clone()),
            ("RSX_GW_JWT_SECRET", self.options.jwt_secret.clone()),
            ("RSX_AUTH_REDIRECT_URI", setup.redirect_uri.clone()),
            ("RSX_AUTH_TRADE_UI_URL", setup.trade_ui_url.clone()),
            (
                "RSX_AUTH_STARTER_COLLATERAL",
                setup.starter_collateral.clone(),
            ),
            ("RSX_AUTH_JWT_TTL_S", setup.jwt_ttl_s.clone()),
            ("DATABASE_URL", self.options.database_url.clone()),
        ]);
        Process {
            name: text(AUTH_NAME),
            binary: text("./rsx-auth/.venv/bin/python"),
            args: vec![text("-m"), text("rsx_auth.app")],
            cwd: text("./rsx-auth"),
            bound: bound_by(&env, &["RSX_AUTH_LISTEN"]),
            env,
        }
    }
}

/// The processes of `scenario` over `symbols` (already selected, in scenario
/// order): matching engines, mark, risk with its replicas, the gateway,
/// marketdata and recorder, then the maker and auth sidecars when the
/// options name them. Pure: the same inputs give the same plan. No symbols
/// is a `Runtime` failure.
pub fn build(
    scenario: &Scenario,
    symbols: &[CatalogSymbol],
    options: &Options,
) -> Result<Plan, Failure> {
    let Some(first) = symbols.first().map(|symbol| symbol.symbol_id) else {
        return Err(Failure::runtime(format!(
            "scenario {} has no symbols",
            scenario.name
        )));
    };
    let target = if options.release {
        "./target/release"
    } else {
        "./target/debug"
    };
    let max_symbols = symbols
        .iter()
        .map(|symbol| symbol.symbol_id)
        .max()
        .map_or(0, |max| max + 1);
    let layout = Layout {
        symbols,
        options,
        target,
        me_cast_addrs: join_symbols(symbols, |symbol| addr(ME_CAST, symbol.symbol_id)),
        risk_me_recv_addrs: join_symbols(symbols, |symbol| {
            format!(
                "{}={}",
                symbol.symbol_id,
                addr(RISK_ME_RECV, symbol.symbol_id)
            )
        }),
        me_replication_addrs: join_symbols(symbols, |symbol| {
            format!(
                "{}={}",
                symbol.symbol_id,
                addr(ME_REPLICATION, symbol.symbol_id)
            )
        }),
        max_symbols,
    };
    let mut processes = Vec::new();
    for symbol in symbols {
        processes.push(layout.matching(symbol));
    }
    processes.push(layout.mark());
    for shard in 0..RISK_SHARDS {
        processes.push(layout.risk_primary(shard));
        for replica in 0..scenario.replication.replica_count() {
            processes.push(layout.risk_replica(shard, replica));
        }
    }
    processes.push(layout.gateway());
    processes.push(layout.marketdata(first));
    processes.push(layout.recorder(first));
    if let Some(tuning) = &options.maker {
        processes.push(layout.maker(tuning));
    }
    if let Some(setup) = &options.auth {
        processes.push(layout.auth(setup));
    }
    Ok(Plan {
        scenario: scenario.name.to_string(),
        processes,
    })
}
