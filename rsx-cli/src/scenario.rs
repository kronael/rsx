use crate::output::Failure;

#[cfg(test)]
#[path = "scenario_test.rs"]
mod scenario_test;

/// Which risk replicas a scenario runs next to the primary shard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Replication {
    None,
    Replica,
    Spare,
}

impl Replication {
    pub fn replica_count(self) -> u32 {
        match self {
            Replication::None => 0,
            Replication::Replica => 1,
            Replication::Spare => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Replication::None => "none",
            Replication::Replica => "replica",
            Replication::Spare => "spare",
        }
    }
}

/// The load a stress scenario drives once the cluster is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Load {
    pub rate: u32,
    pub duration_s: u32,
    pub kind: &'static str,
}

/// One named cluster shape: how many symbols (taken in `SYMBOL_ORDER`), how
/// many gateways and which risk replicas.
#[derive(Debug, PartialEq, Eq)]
pub struct Scenario {
    pub name: &'static str,
    pub symbols: usize,
    pub replication: Replication,
    pub gateways: u32,
    pub load: Option<Load>,
}

/// The order scenarios take symbols in; names resolve against Postgres.
pub const SYMBOL_ORDER: [&str; 8] = ["PENGU", "SOL", "BTC", "ETH", "WIF", "BONK", "PEPE", "DOGE"];

const fn shape(
    name: &'static str,
    symbols: usize,
    replication: Replication,
    gateways: u32,
) -> Scenario {
    Scenario {
        name,
        symbols,
        replication,
        gateways,
        load: None,
    }
}

const fn stress(
    name: &'static str,
    symbols: usize,
    replication: Replication,
    gateways: u32,
    load: Load,
) -> Scenario {
    Scenario {
        name,
        symbols,
        replication,
        gateways,
        load: Some(load),
    }
}

pub const SCENARIOS: [Scenario; 15] = [
    shape("minimal", 1, Replication::None, 1),
    shape("1Z", 1, Replication::None, 1),
    shape("standard", 1, Replication::Replica, 1),
    shape("1", 1, Replication::Replica, 1),
    shape("duo", 2, Replication::None, 1),
    shape("2Z", 2, Replication::None, 1),
    shape("full", 3, Replication::Replica, 1),
    shape("3", 3, Replication::Replica, 1),
    shape("trio", 3, Replication::None, 1),
    shape("3Z", 3, Replication::None, 1),
    shape("stress", 3, Replication::Spare, 2),
    shape("M3S", 3, Replication::Spare, 2),
    stress(
        "stress-low",
        3,
        Replication::None,
        1,
        Load {
            rate: 10,
            duration_s: 60,
            kind: "sustained",
        },
    ),
    stress(
        "stress-high",
        3,
        Replication::Replica,
        1,
        Load {
            rate: 100,
            duration_s: 60,
            kind: "sustained",
        },
    ),
    stress(
        "stress-ultra",
        4,
        Replication::Replica,
        2,
        Load {
            rate: 500,
            duration_s: 10,
            kind: "burst",
        },
    ),
];

/// The scenario called `name`, or a `Misuse` failure listing the known names.
pub fn find(name: &str) -> Result<&'static Scenario, Failure> {
    SCENARIOS
        .iter()
        .find(|scenario| scenario.name == name)
        .ok_or_else(|| {
            let known: Vec<&str> = SCENARIOS.iter().map(|scenario| scenario.name).collect();
            Failure::misuse(format!(
                "unknown scenario {name:?} (known: {})",
                known.join(", ")
            ))
        })
}
