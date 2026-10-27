use crate::catalog::redact;
use crate::cluster::do_down;
use crate::cluster::print_report;
use crate::cluster::Report;
use crate::output::Failure;
use crate::output::Outcome;
use crate::runtime_utils::block_on;
use rsx_risk::db::connect;
use rsx_risk::schema::run_migrations;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
#[path = "reset_test.rs"]
mod reset_test;

/// Every stream's WAL, and the archive, live under this directory.
const WAL_DIR: &str = "tmp/wal";
/// Replay checkpoints of marketdata and the recorder, in `tmp/`.
const MARKETDATA_TIP: &str = "md.tip";
const RECORDER_TIP_PREFIX: &str = "recorder-tip-";

/// Risk's per-run Postgres state: positions, fills, funding and the `tips`
/// replication resume-seq checkpoint. `accounts` (seeded collateral), `users`
/// and the symbol tables are kept.
pub const RISK_STATE_TABLES: [&str; 8] = [
    "tips",
    "positions",
    "fills",
    "funding",
    "funding_intervals",
    "insurance_fund",
    "liquidations",
    "frozen_orders",
];

/// One statement that truncates `RISK_STATE_TABLES`, skipping any table the
/// schema does not have.
pub fn truncate_sql() -> String {
    let tables = RISK_STATE_TABLES
        .iter()
        .map(|table| format!("'{table}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "DO $$ DECLARE t text; BEGIN \
         FOREACH t IN ARRAY ARRAY[{tables}] LOOP \
         IF EXISTS (SELECT 1 FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name = t) THEN \
         EXECUTE format('TRUNCATE TABLE %I RESTART IDENTITY', t); \
         END IF; END LOOP; END $$;"
    )
}

/// Stops what the cluster started, truncates risk's Postgres state, then
/// deletes the WAL and the replay checkpoints, so the next `up` boots every
/// component at seq 0 together. Postgres goes first: a failure leaves the WAL
/// as it is and the two stay consistent.
pub fn reset(root: &Path, database_url: &str, is_json: bool) -> Outcome {
    let mut report = do_down(root, &[])?;
    block_on(truncate_risk_state(database_url))?;
    report.cleared.push("risk-state".to_string());
    wipe_files(root, &mut report)?;
    print_report(&report, is_json);
    Ok(())
}

async fn truncate_risk_state(database_url: &str) -> Result<(), Failure> {
    let shown = redact(database_url);
    let client = connect(database_url).await.map_err(|error| {
        Failure::runtime(format!(
            "postgres is not reachable at {shown}: {error} (start it, or pass --db); \
             the processes are stopped and the WAL is untouched"
        ))
    })?;
    run_migrations(&client)
        .await
        .map_err(|error| Failure::runtime(format!("migrations failed at {shown}: {error}")))?;
    client
        .batch_execute(&truncate_sql())
        .await
        .map_err(|error| Failure::runtime(format!("failed to truncate risk state: {error}")))
}

/// Deletes the children of `<root>/tmp/wal` (the directory stays), the
/// marketdata tip and the recorder tips, noting in `report` what it cleared.
fn wipe_files(root: &Path, report: &mut Report) -> Result<(), Failure> {
    let wal = root.join(WAL_DIR);
    if wal.is_dir() {
        for entry in read_directory(&wal)? {
            remove_entry(&entry)?;
        }
        report.cleared.push("wal".to_string());
    }
    let tmp = root.join("tmp");
    let mut tips = Vec::new();
    if tmp.is_dir() {
        for entry in read_directory(&tmp)? {
            let is_tip = entry.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name == MARKETDATA_TIP || name.starts_with(RECORDER_TIP_PREFIX)
            });
            if is_tip {
                tips.push(entry);
            }
        }
    }
    for tip in &tips {
        remove_entry(tip)?;
    }
    if !tips.is_empty() {
        report.cleared.push("tips".to_string());
    }
    Ok(())
}

fn read_directory(directory: &Path) -> Result<Vec<PathBuf>, Failure> {
    let failed = |error: std::io::Error| {
        Failure::runtime(format!("failed to read {}: {error}", directory.display()))
    };
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory).map_err(failed)? {
        entries.push(entry.map_err(failed)?.path());
    }
    entries.sort();
    Ok(entries)
}

fn remove_entry(path: &Path) -> Result<(), Failure> {
    let result = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result
        .map_err(|error| Failure::runtime(format!("failed to remove {}: {error}", path.display())))
}
