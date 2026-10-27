use crate::cluster::Report;
use crate::reset::reset;
use crate::reset::truncate_sql;
use crate::reset::wipe_files;
use crate::reset::RISK_STATE_TABLES;
use std::fs;
use tempfile::tempdir;

#[test]
fn wipe_files_deletes_the_wal_children_and_the_tips_but_keeps_the_rest() {
    let root = tempdir().unwrap();
    let tmp = root.path().join("tmp");
    fs::create_dir_all(tmp.join("wal/pengu")).unwrap();
    fs::create_dir_all(tmp.join("wal/archive/1")).unwrap();
    fs::write(tmp.join("wal/pengu/1.wal"), "x").unwrap();
    fs::write(tmp.join("wal/loose.bin"), "x").unwrap();
    fs::write(tmp.join("md.tip"), "1").unwrap();
    fs::write(tmp.join("recorder-tip-1"), "1").unwrap();
    fs::write(tmp.join("recorder-tip-2"), "1").unwrap();
    fs::write(tmp.join("maker-config.json"), "{}").unwrap();
    fs::create_dir_all(tmp.join("cluster/pids")).unwrap();

    let mut report = Report::default();
    wipe_files(root.path(), &mut report).unwrap();

    assert_eq!(report.cleared, ["wal", "tips"]);
    assert!(tmp.join("wal").is_dir());
    assert_eq!(fs::read_dir(tmp.join("wal")).unwrap().count(), 0);
    assert!(!tmp.join("md.tip").exists());
    assert!(!tmp.join("recorder-tip-1").exists());
    assert!(!tmp.join("recorder-tip-2").exists());
    assert!(tmp.join("maker-config.json").exists());
    assert!(tmp.join("cluster/pids").is_dir());
}

#[test]
fn wipe_files_on_a_clean_root_clears_nothing() {
    let root = tempdir().unwrap();
    let mut report = Report::default();
    wipe_files(root.path(), &mut report).unwrap();
    assert!(report.cleared.is_empty());
}

#[test]
fn truncate_sql_names_every_risk_state_table_and_no_kept_one() {
    let sql = truncate_sql();
    for table in RISK_STATE_TABLES {
        assert!(sql.contains(&format!("'{table}'")), "{table}");
    }
    for kept in [
        "accounts",
        "users",
        "symbol_static",
        "symbol_config_schedule",
    ] {
        assert!(!sql.contains(kept), "{kept}");
    }
    assert!(sql.contains("information_schema.tables"));
}

#[test]
fn reset_leaves_the_wal_when_postgres_is_unreachable() {
    let root = tempdir().unwrap();
    let wal = root.path().join("tmp/wal/pengu");
    fs::create_dir_all(&wal).unwrap();
    fs::write(wal.join("1.wal"), "x").unwrap();

    let failure = reset(root.path(), "postgres://nobody@127.0.0.1:1/none", true).unwrap_err();

    assert!(
        failure.message().contains("not reachable"),
        "{}",
        failure.message()
    );
    assert!(failure.message().contains("WAL is untouched"));
    assert!(wal.join("1.wal").exists());
}
