use crate::check::Check;
use crate::check::Report;
use crate::check::Status;
use crate::check::Tally;
use crate::check::MAX_VIOLATIONS;
use serde_json::json;

fn tally(checked: u64, violations: u64) -> Tally {
    let mut tally = Tally {
        checked,
        ..Tally::default()
    };
    for index in 0..violations {
        tally.add_violation(json!({ "index": index }));
    }
    tally
}

#[test]
fn a_violation_fails_the_check() {
    let check = Check::from_tally(Some(1), "name", "fills", tally(5, 1));
    assert_eq!(check.status, Status::Fail);
    assert_eq!(check.violation_count, 1);
}

#[test]
fn a_check_that_examined_nothing_is_skipped_not_passed() {
    let check = Check::from_tally(Some(1), "name", "fills", tally(0, 0));
    assert_eq!(check.status, Status::Skip);
    assert_eq!(check.detail, "no fills to check");
}

#[test]
fn clean_examined_records_pass() {
    let check = Check::from_tally(Some(1), "name", "fills", tally(3, 0));
    assert_eq!(check.status, Status::Pass);
    assert_eq!(check.checked, 3);
}

#[test]
fn only_the_first_violations_are_kept_but_all_are_counted() {
    let check = Check::from_tally(Some(1), "name", "fills", tally(50, 25));
    assert_eq!(check.violation_count, 25);
    assert_eq!(check.violations.len(), MAX_VIOLATIONS);
    assert_eq!(check.violations[0], json!({ "index": 0 }));
}

#[test]
fn merging_keeps_the_cap_and_the_counts() {
    let mut left = tally(10, 8);
    left.merge(&tally(5, 8));
    assert_eq!(left.checked, 15);
    assert_eq!(left.violation_count, 16);
    assert_eq!(left.violations.len(), MAX_VIOLATIONS);
}

#[test]
fn one_failed_check_fails_the_report_and_a_skip_does_not() {
    let passing = Report {
        checks: vec![
            Check::from_tally(Some(1), "a", "x", tally(1, 0)),
            Check::skip(Some(7), "b", "reason"),
        ],
    };
    assert!(!passing.is_failed());
    assert_eq!(passing.to_json()["status"], "pass");

    let failing = Report {
        checks: vec![
            Check::from_tally(Some(1), "a", "x", tally(1, 0)),
            Check::fail(None, "b", "boom"),
        ],
    };
    assert!(failing.is_failed());
    assert_eq!(failing.failed_count(), 1);
    assert_eq!(failing.to_json()["status"], "fail");
}

#[test]
fn a_report_where_nothing_ran_is_skipped_not_passed() {
    let report = Report {
        checks: vec![
            Check::skip(Some(7), "a", "reason"),
            Check::from_tally(Some(1), "b", "x", tally(0, 0)),
        ],
    };
    assert!(!report.is_failed());
    assert_eq!(report.to_json()["status"], "skip");
}
