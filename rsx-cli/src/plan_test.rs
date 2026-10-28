use crate::plan::build;
use crate::plan::select_symbols;
use crate::plan::AuthSetup;
use crate::plan::CatalogSymbol;
use crate::plan::Layout;
use crate::plan::MakerTuning;
use crate::plan::Options;
use crate::plan::Plan;
use crate::plan::Process;
use crate::scenario::find;
use crate::scenario::SCENARIOS;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;

fn symbol(
    symbol_id: u32,
    name: &str,
    tick_size: i64,
    lot_size: i64,
    price_decimals: u32,
    qty_decimals: u32,
) -> CatalogSymbol {
    CatalogSymbol {
        symbol_id,
        name: name.to_string(),
        tick_size,
        lot_size,
        price_decimals,
        qty_decimals,
    }
}

fn migration_006_catalog() -> Vec<CatalogSymbol> {
    vec![
        symbol(1, "BTC", 50, 100, 2, 8),
        symbol(2, "ETH", 10, 1000, 2, 8),
        symbol(3, "SOL", 1, 10000, 4, 6),
        symbol(10, "PENGU", 1, 100000, 6, 4),
        symbol(11, "WIF", 1, 100000, 6, 4),
        symbol(12, "BONK", 1, 100000, 6, 4),
        symbol(13, "PEPE", 1, 100000, 6, 4),
        symbol(14, "DOGE", 1, 100000, 6, 4),
    ]
}

fn plan_of(scenario_name: &str, options: &Options) -> Plan {
    let scenario = find(scenario_name).unwrap();
    let symbols = select_symbols(scenario, &migration_006_catalog()).unwrap();
    build(scenario, &symbols, options).unwrap()
}

fn golden(file: &str) -> Value {
    let path = format!(
        "{}/tests/golden/cluster/{file}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap()
}

fn assert_matches_golden(plan: &Plan, file: &str) {
    let expected = golden(file);
    let expected = expected["processes"].as_array().unwrap();
    let names: Vec<&str> = plan
        .processes
        .iter()
        .map(|process| process.name.as_str())
        .collect();
    let expected_names: Vec<&str> = expected
        .iter()
        .map(|process| process["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, expected_names, "{file}: process order");
    for (process, expected) in plan.processes.iter().zip(expected) {
        assert_eq!(
            process.binary,
            expected["binary"].as_str().unwrap(),
            "{file}: {} binary",
            process.name
        );
        let want: BTreeMap<String, String> =
            serde_json::from_value(expected["env"].clone()).unwrap();
        assert_eq!(process.env, want, "{file}: {} env", process.name);
    }
}

#[test]
fn every_scenario_plan_equals_the_playground_runtime_plan() {
    for scenario in &SCENARIOS {
        let plan = plan_of(scenario.name, &Options::default());
        assert_matches_golden(&plan, scenario.name);
    }
}

#[test]
fn pinned_trio_equals_the_playground_runtime_plan() {
    let options = Options {
        pin_cores: true,
        ..Options::default()
    };
    let plan = plan_of("trio", &options);
    assert_matches_golden(&plan, "trio-pinned");
    assert_eq!(plan.max_core_id(), Some(5));
}

#[test]
fn release_stress_equals_the_playground_runtime_plan() {
    let options = Options {
        release: true,
        ..Options::default()
    };
    let plan = plan_of("stress", &options);
    assert_matches_golden(&plan, "stress-release");
}

#[test]
fn an_unpinned_plan_sets_no_core_id() {
    let plan = plan_of("full", &Options::default());
    assert_eq!(plan.max_core_id(), None);
}

#[test]
fn replicas_follow_the_scenario() {
    let names = |scenario_name: &str| -> Vec<String> {
        plan_of(scenario_name, &Options::default())
            .processes
            .iter()
            .filter(|process| process.is_replica())
            .map(|process| process.name.clone())
            .collect()
    };
    assert!(names("minimal").is_empty());
    assert_eq!(names("standard"), ["risk-0-replica-0"]);
    assert_eq!(names("stress"), ["risk-0-replica-0", "risk-0-replica-1"]);
}

#[test]
fn ports_are_the_ones_each_process_binds() {
    let plan = plan_of("minimal", &Options::default());
    let ports = |name: &str| plan.process(name).unwrap().ports();
    assert_eq!(ports("me-pengu"), [9110, 9710, 9810]);
    assert_eq!(ports("mark"), [9400, 10100]);
    assert_eq!(ports("risk-0"), [9200, 9600, 9900, 10400, 28310]);
    assert_eq!(ports("gw-0"), [8088, 9300, 10000]);
    assert_eq!(ports("marketdata"), [8180, 9510, 10200]);
    assert_eq!(ports("recorder"), [10300]);
}

#[test]
fn a_sidecar_binds_what_it_listens_on() {
    let options = Options {
        maker: Some(MakerTuning::default()),
        auth: Some(AuthSetup::default()),
        ..Options::default()
    };
    let plan = plan_of("minimal", &options);
    assert_eq!(plan.process("auth").unwrap().ports(), [8082]);
    assert!(plan.process("maker").unwrap().ports().is_empty());
}

#[test]
fn risk_health_ports_do_not_overlap_across_shards_and_replicas() {
    let plan = plan_of("stress", &Options::default());
    let mut health: Vec<&str> = plan
        .processes
        .iter()
        .filter(|process| process.name.starts_with("risk-"))
        .filter_map(Process::health_addr)
        .collect();
    let total = health.len();
    health.sort_unstable();
    health.dedup();
    assert_eq!(health.len(), total);
    let replica = plan.process("risk-0-replica-1").unwrap();
    assert_eq!(replica.health_addr(), Some("127.0.0.1:9951"));
    assert_eq!(replica.env["RSX_RISK_CAST_ADDR"], "127.0.0.1:9251");
    assert_eq!(replica.env["RSX_RISK_MARK_CAST_ADDR"], "127.0.0.1:9651");
}

#[test]
fn risk_addresses_stay_unique_past_ten_shards() {
    let options = Options::default();
    let layout = Layout {
        symbols: &[],
        options: &options,
        target: "./target/debug",
        me_cast_addrs: String::new(),
        risk_me_recv_addrs: String::new(),
        max_symbols: 0,
        me_replication_addrs: String::new(),
    };
    let mut risk = Vec::new();
    for shard in 0..12 {
        risk.push(layout.risk_primary(shard));
        for replica in 0..2 {
            risk.push(layout.risk_replica(shard, replica));
        }
    }
    let bound = |key: &str, only_replicas: bool| -> Vec<String> {
        risk.iter()
            .filter(|process| !only_replicas || process.is_replica() || process.name == "risk-0")
            .map(|process| process.env[key].clone())
            .collect()
    };
    for (key, only_replicas) in [
        ("RSX_RISK_CAST_ADDR", false),
        ("RSX_RISK_HEALTH_ADDR", false),
        ("RSX_RISK_MARK_CAST_ADDR", true),
    ] {
        let mut addrs = bound(key, only_replicas);
        let total = addrs.len();
        addrs.sort_unstable();
        addrs.dedup();
        assert_eq!(addrs.len(), total, "{key} repeats");
    }
}

#[test]
fn health_addr_names_each_daemons_health_server() {
    let plan = plan_of("minimal", &Options::default());
    assert_eq!(
        plan.process("me-pengu").unwrap().health_addr(),
        Some("127.0.0.1:9810")
    );
    assert_eq!(
        plan.process("gw-0").unwrap().health_addr(),
        Some("127.0.0.1:10000")
    );
    assert_eq!(
        plan.process("risk-0").unwrap().health_addr(),
        Some("127.0.0.1:9900")
    );
}

#[test]
fn maker_quotes_every_scenario_symbol_with_the_saved_tuning() {
    let tuning = MakerTuning::from_json(r#"{"spread_bps": 35, "levels": 3, "junk": "x"}"#);
    let options = Options {
        maker: Some(tuning),
        ..Options::default()
    };
    let plan = plan_of("trio", &options);
    let maker = plan.processes.last().unwrap();
    assert_eq!(maker.name, "maker");
    assert_eq!(maker.binary, "./rsx-maker/rsx-maker");
    assert_eq!(maker.env["RSX_MAKER_SYMBOLS"], "10,3,1");
    assert_eq!(maker.env["RSX_MAKER_SPREAD_BPS"], "35");
    assert_eq!(maker.env["RSX_MAKER_LEVELS"], "3");
    assert_eq!(maker.env["RSX_MAKER_QTY"], "10");
    assert_eq!(maker.env["GATEWAY_URL"], "ws://127.0.0.1:8088");
    assert_eq!(
        maker.env["RSX_GW_JWT_SECRET"],
        plan.process("gw-0").unwrap().env["RSX_GW_JWT_SECRET"]
    );
}

#[test]
fn malformed_maker_config_keeps_the_defaults() {
    assert_eq!(MakerTuning::from_json("not json"), MakerTuning::default());
    assert_eq!(MakerTuning::from_json("[1]"), MakerTuning::default());
}

#[test]
fn auth_runs_the_venv_python_in_its_own_directory() {
    let options = Options {
        auth: Some(AuthSetup::default()),
        ..Options::default()
    };
    let plan = plan_of("minimal", &options);
    let auth = plan.process("auth").unwrap();
    assert_eq!(auth.binary, "./rsx-auth/.venv/bin/python");
    assert_eq!(auth.args, ["-m", "rsx_auth.app"]);
    assert_eq!(auth.cwd, "./rsx-auth");
    assert!(!auth.env.contains_key("RSX_AUTH_GITHUB_CLIENT_SECRET"));
    assert_eq!(auth.health_addr(), None);
}

#[test]
fn a_missing_symbol_names_the_scenario_and_the_symbol() {
    let catalog: Vec<CatalogSymbol> = migration_006_catalog()
        .into_iter()
        .filter(|symbol| symbol.name != "SOL")
        .collect();
    let failure = select_symbols(find("trio").unwrap(), &catalog).unwrap_err();
    assert!(failure.message().contains("trio"));
    assert!(failure.message().contains("SOL"));
}

#[test]
fn no_symbols_is_a_failure() {
    let failure = build(find("minimal").unwrap(), &[], &Options::default()).unwrap_err();
    assert!(failure.message().contains("no symbols"));
}

#[test]
fn plan_json_carries_the_contract_keys() {
    let json = plan_of("minimal", &Options::default()).to_json();
    assert_eq!(json["scenario"], "minimal");
    let process = &json["processes"][0];
    let mut keys: Vec<&str> = process
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["args", "binary", "cwd", "env", "name", "ports"]);
}
