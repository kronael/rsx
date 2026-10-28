use std::process::Command;
use std::process::Stdio;
use tempfile::TempDir;

/// With `RSX_ME_DATABASE_URL` set, an unreachable Postgres stops boot: the
/// applied config cannot be read, and the env config may be stale.
#[test]
fn unreachable_postgres_stops_boot() {
    let wal_dir = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rsx-matching"))
        .env("RSX_ME_SYMBOL_ID", "1")
        .env("RSX_ME_PRICE_DECIMALS", "2")
        .env("RSX_ME_QTY_DECIMALS", "3")
        .env("RSX_ME_TICK_SIZE", "1")
        .env("RSX_ME_LOT_SIZE", "1")
        .env("RSX_ME_WAL_DIR", wal_dir.path())
        // Port 1 refuses connections immediately.
        .env(
            "RSX_ME_DATABASE_URL",
            "host=127.0.0.1 port=1 user=postgres connect_timeout=5",
        )
        .stdin(Stdio::null())
        .output()
        .expect("run rsx-matching");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("fatal: boot config"), "{stderr}");
    assert!(
        std::fs::read_dir(wal_dir.path()).unwrap().next().is_none(),
        "boot stopped before the WAL was opened"
    );
}

/// An address that never answers: each of the 12 attempts times out after
/// 5 s and 11 pauses of 1 s separate them, so boot stops within about 70 s.
/// Ignored because it waits out that bound.
#[test]
#[ignore]
fn silent_postgres_stops_boot_within_the_connect_bound() {
    let wal_dir = TempDir::new().unwrap();
    let started = std::time::Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_rsx-matching"))
        .env("RSX_ME_SYMBOL_ID", "1")
        .env("RSX_ME_PRICE_DECIMALS", "2")
        .env("RSX_ME_QTY_DECIMALS", "3")
        .env("RSX_ME_TICK_SIZE", "1")
        .env("RSX_ME_LOT_SIZE", "1")
        .env("RSX_ME_WAL_DIR", wal_dir.path())
        .env(
            "RSX_ME_DATABASE_URL",
            "host=10.255.255.1 port=5432 user=postgres",
        )
        .stdin(Stdio::null())
        .output()
        .expect("run rsx-matching");

    assert_eq!(output.status.code(), Some(1));
    assert!(started.elapsed() < std::time::Duration::from_secs(75));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("fatal: boot config"), "{stderr}");
}
