use crate::output::Failure;

#[test]
fn test_runtime_failure_exits_1() {
    let failure = Failure::runtime("failed to open wal: gone");
    assert_eq!(failure.exit_code(), 1);
    assert_eq!(failure.message(), "failed to open wal: gone");
}

#[test]
fn test_misuse_failure_exits_2() {
    let failure = Failure::misuse("unknown record type: NOPE");
    assert_eq!(failure.exit_code(), 2);
    assert_eq!(failure.message(), "unknown record type: NOPE");
}
