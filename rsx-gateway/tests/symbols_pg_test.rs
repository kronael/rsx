//! The gateway's symbol table from risk's Postgres config: boot load,
//! active-version rule, malformed rows, new versions picked up, and the
//! boot failures.

#[path = "common/postgres.rs"]
mod postgres;

use postgres::TestPostgres;
use rsx_gateway::symbols::boot_load;
use rsx_gateway::symbols::spawn_sidecar;
use rsx_gateway::symbols::SymbolTable;
use std::io::ErrorKind;
use std::time::Duration;
use std::time::Instant;

const NO_WAIT: Duration = Duration::ZERO;
const DEADLINE: Duration = Duration::from_secs(20);

const NEW_PENGU_VERSION: &str = "INSERT INTO symbol_config_schedule (symbol_id, config_version, \
     effective_at_ms, tick_size, lot_size, price_decimals, qty_decimals, status, min_notional, \
     created_at_ms) VALUES (10, 3, 0, 1, 200000, 6, 4, 'trading', 10000000000, 0)";

fn wait_for_table(
    updates: &mut rtrb::Consumer<SymbolTable>,
    is_wanted: impl Fn(&SymbolTable) -> bool,
) -> SymbolTable {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if let Ok(table) = updates.pop() {
            if is_wanted(&table) {
                return table;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("no wanted symbol table within {DEADLINE:?}");
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn boot_reads_the_active_row_of_every_seeded_symbol() {
    let postgres = TestPostgres::start_migrated();
    let source = boot_load(&postgres.url, NO_WAIT).unwrap();
    assert_eq!(source.table.len(), 8);
    let pengu = &source.table[&10];
    assert_eq!(pengu.config.price_decimals, 6);
    assert_eq!(pengu.config.qty_decimals, 4);
    assert_eq!(pengu.config.tick_size, 1);
    assert_eq!(pengu.config.lot_size, 100_000);
    assert_eq!(pengu.status, "trading");
    assert_eq!(pengu.min_notional, 10_000_000_000);
    assert_eq!(pengu.config_version, 2);
    let bitcoin = &source.table[&1];
    assert_eq!(bitcoin.config.price_decimals, 2);
    assert_eq!(bitcoin.config.qty_decimals, 8);
    assert!(!source.table.contains_key(&0));
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn active_row_is_the_highest_version_already_effective() {
    let postgres = TestPostgres::start_migrated();
    postgres.execute(NEW_PENGU_VERSION);
    postgres.execute(
        "INSERT INTO symbol_config_schedule (symbol_id, config_version, effective_at_ms, \
         tick_size, lot_size, price_decimals, qty_decimals, status, min_notional, created_at_ms) \
         VALUES (10, 4, 99999999999999, 1, 300000, 6, 4, 'trading', 10000000000, 0)",
    );
    let source = boot_load(&postgres.url, NO_WAIT).unwrap();
    let pengu = &source.table[&10];
    assert_eq!(pengu.config_version, 3);
    assert_eq!(pengu.config.lot_size, 200_000);
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn a_malformed_row_fails_the_load() {
    let postgres = TestPostgres::start_migrated();
    postgres.execute(NEW_PENGU_VERSION);
    for broken in [
        "tick_size = 0",
        "lot_size = -1",
        "min_notional = NULL",
        "min_notional = 0",
        "status = 'Not A Word'",
        "price_decimals = 300",
    ] {
        postgres.execute(&format!(
            "UPDATE symbol_config_schedule SET {broken} \
             WHERE symbol_id = 10 AND config_version = 3"
        ));
        let error = boot_load(&postgres.url, NO_WAIT)
            .err()
            .unwrap_or_else(|| panic!("{broken} was accepted"));
        assert_eq!(error.kind(), ErrorKind::InvalidData, "{broken}: {error}");
        postgres.execute(
            "UPDATE symbol_config_schedule SET tick_size = 1, lot_size = 200000, \
             min_notional = 10000000000, status = 'trading', price_decimals = 6 \
             WHERE symbol_id = 10 AND config_version = 3",
        );
    }
    assert!(boot_load(&postgres.url, NO_WAIT).is_ok());
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn a_new_version_is_picked_up_on_the_poll() {
    let postgres = TestPostgres::start_migrated();
    let source = boot_load(&postgres.url, NO_WAIT).unwrap();
    let mut sidecar = spawn_sidecar(source, Duration::from_millis(100)).unwrap();
    postgres.execute(NEW_PENGU_VERSION);
    let table = wait_for_table(&mut sidecar.updates, |table| table[&10].config_version == 3);
    assert_eq!(table[&10].config.lot_size, 200_000);
    assert_eq!(table[&1].config_version, 2);
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn a_reload_request_picks_up_a_new_version_before_the_poll() {
    let postgres = TestPostgres::start_migrated();
    let source = boot_load(&postgres.url, NO_WAIT).unwrap();
    let mut sidecar = spawn_sidecar(source, Duration::from_secs(3600)).unwrap();
    postgres.execute(NEW_PENGU_VERSION);
    sidecar.reload.notify_one();
    let table = wait_for_table(&mut sidecar.updates, |table| table[&10].config_version == 3);
    assert_eq!(table[&10].config.lot_size, 200_000);
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn an_unchanged_table_is_not_published() {
    let postgres = TestPostgres::start_migrated();
    let source = boot_load(&postgres.url, NO_WAIT).unwrap();
    let mut sidecar = spawn_sidecar(source, Duration::from_millis(50)).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(sidecar.updates.pop().is_err());
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn a_schema_that_risk_has_not_created_fails_after_the_wait() {
    let postgres = TestPostgres::start();
    let error = boot_load(&postgres.url, NO_WAIT).err().unwrap();
    assert_eq!(error.kind(), ErrorKind::NotFound, "{error}");
}

#[test]
#[ignore = "needs a docker daemon (make integration)"]
fn boot_waits_for_risk_to_migrate() {
    let postgres = TestPostgres::start();
    let url = postgres.url.clone();
    let boot = std::thread::spawn(move || boot_load(&url, Duration::from_secs(30)));
    std::thread::sleep(Duration::from_millis(800));
    postgres.migrate();
    let source = boot.join().unwrap().unwrap();
    assert_eq!(source.table.len(), 8);
}

#[test]
fn an_unreachable_postgres_fails_the_boot() {
    let error = boot_load(
        "host=127.0.0.1 port=1 user=postgres connect_timeout=2",
        Duration::from_secs(30),
    )
    .err()
    .unwrap();
    assert_ne!(error.kind(), ErrorKind::NotFound, "{error}");
}
