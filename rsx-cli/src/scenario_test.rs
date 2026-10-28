use crate::scenario::find;
use crate::scenario::Replication;
use crate::scenario::SCENARIOS;
use crate::scenario::SYMBOL_ORDER;

#[test]
fn every_scenario_name_is_unique_and_findable() {
    for scenario in &SCENARIOS {
        assert_eq!(find(scenario.name).unwrap(), scenario);
    }
    let mut names: Vec<&str> = SCENARIOS.iter().map(|scenario| scenario.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), SCENARIOS.len());
}

#[test]
fn no_scenario_wants_more_symbols_than_the_order_lists() {
    for scenario in &SCENARIOS {
        assert!(scenario.symbols >= 1 && scenario.symbols <= SYMBOL_ORDER.len());
    }
}

#[test]
fn unknown_scenario_is_misuse_listing_the_known_names() {
    let failure = find("nope").unwrap_err();
    assert_eq!(failure.exit_code(), 2);
    assert!(failure.message().contains("minimal"));
    assert!(failure.message().contains("stress-ultra"));
}

#[test]
fn shapes_match_the_playground_table() {
    let stress = find("stress").unwrap();
    assert_eq!(stress.replication, Replication::Spare);
    assert_eq!(stress.replication.replica_count(), 2);
    let ultra = find("stress-ultra").unwrap();
    assert_eq!(ultra.symbols, 4);
    let load = ultra.load.unwrap();
    assert_eq!((load.rate, load.duration_s, load.kind), (500, 10, "burst"));
    assert!(find("minimal").unwrap().load.is_none());
}
