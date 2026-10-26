use serde_json::json;
use serde_json::Value;

#[cfg(test)]
#[path = "check_test.rs"]
mod check_test;

pub const INTACT: &str = "WAL records are intact";
pub const DATABASE: &str = "Postgres is reachable";
pub const FILLS: &str = "Fills precede ORDER_DONE";
pub const COMPLETION: &str = "Every order completes exactly once";
pub const FIFO: &str = "Fills follow time priority within a price level";
pub const REPLAY_MATCH: &str = "Replayed book matches the WAL";
pub const POSITIONS: &str = "Position equals the sum of fills";
pub const SEQS: &str = "WAL seqs strictly increase";
pub const TIPS: &str = "Risk tips stay within the ME WAL";
pub const BBO: &str = "BBO records are not crossed";
pub const REPLAY_CROSSED: &str = "Replayed book is not crossed";
pub const SPSC: &str = "SPSC rings preserve FIFO";
pub const SLAB: &str = "Slab has no leak";
pub const FUNDING: &str = "Funding sums to zero per symbol and interval";
pub const LOCKS: &str = "At most one holder per shard lock";

/// Offending records kept per check; the count keeps the rest.
pub const MAX_VIOLATIONS: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Skip,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Skip => "skip",
        }
    }
}

/// What one check examined and the first records that broke it.
#[derive(Default)]
pub struct Tally {
    pub checked: u64,
    pub violation_count: u64,
    pub violations: Vec<Value>,
}

impl Tally {
    pub fn add_violation(&mut self, violation: Value) {
        self.violation_count += 1;
        if self.violations.len() < MAX_VIOLATIONS {
            self.violations.push(violation);
        }
    }

    pub fn merge(&mut self, other: &Tally) {
        self.checked += other.checked;
        self.violation_count += other.violation_count;
        for violation in &other.violations {
            if self.violations.len() < MAX_VIOLATIONS {
                self.violations.push(violation.clone());
            }
        }
    }
}

/// One entry of the report: an invariant or a precondition and its verdict.
pub struct Check {
    pub invariant: Option<u8>,
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
    pub checked: u64,
    pub violation_count: u64,
    pub violations: Vec<Value>,
}

impl Check {
    /// `Fail` on any violation, `Skip` when nothing was examined (a check
    /// that examined nothing proved nothing), else `Pass`. `noun` names what
    /// was examined, for the detail line.
    pub fn from_tally(
        invariant: Option<u8>,
        name: &'static str,
        noun: &str,
        tally: Tally,
    ) -> Check {
        let (status, detail) = if tally.violation_count > 0 {
            (
                Status::Fail,
                format!(
                    "{} violations in {} {} checked",
                    tally.violation_count, tally.checked, noun
                ),
            )
        } else if tally.checked == 0 {
            (Status::Skip, format!("no {} to check", noun))
        } else {
            (Status::Pass, format!("{} {} checked", tally.checked, noun))
        };
        Check {
            invariant,
            name,
            status,
            detail,
            checked: tally.checked,
            violation_count: tally.violation_count,
            violations: tally.violations,
        }
    }

    pub fn pass(invariant: Option<u8>, name: &'static str, detail: impl Into<String>) -> Check {
        Check {
            invariant,
            name,
            status: Status::Pass,
            detail: detail.into(),
            checked: 0,
            violation_count: 0,
            violations: Vec::new(),
        }
    }

    pub fn skip(invariant: Option<u8>, name: &'static str, reason: impl Into<String>) -> Check {
        Check {
            invariant,
            name,
            status: Status::Skip,
            detail: reason.into(),
            checked: 0,
            violation_count: 0,
            violations: Vec::new(),
        }
    }

    /// A check that could not examine its source: one violation, the message.
    pub fn fail(invariant: Option<u8>, name: &'static str, message: impl Into<String>) -> Check {
        let message = message.into();
        Check {
            invariant,
            name,
            status: Status::Fail,
            detail: message.clone(),
            checked: 0,
            violation_count: 1,
            violations: vec![json!({ "message": message })],
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "invariant": self.invariant,
            "name": self.name,
            "status": self.status.as_str(),
            "detail": self.detail,
            "checked": self.checked,
            "violation_count": self.violation_count,
            "violations": self.violations,
        })
    }

    pub fn to_text(&self) -> String {
        let invariant = self
            .invariant
            .map_or("-".to_string(), |number| number.to_string());
        let mut text = format!(
            "{:<5} {:>2}  {}: {}",
            self.status.as_str().to_uppercase(),
            invariant,
            self.name,
            self.detail
        );
        for violation in &self.violations {
            text.push_str(&format!("\n          {}", violation));
        }
        text
    }
}

pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    /// `Fail` when any check failed, `Pass` when at least one passed and
    /// none failed, `Skip` when no check could run.
    pub fn status(&self) -> Status {
        if self.checks.iter().any(|check| check.status == Status::Fail) {
            Status::Fail
        } else if self.checks.iter().any(|check| check.status == Status::Pass) {
            Status::Pass
        } else {
            Status::Skip
        }
    }

    pub fn is_failed(&self) -> bool {
        self.status() == Status::Fail
    }

    pub fn failed_count(&self) -> usize {
        self.checks
            .iter()
            .filter(|check| check.status == Status::Fail)
            .count()
    }

    pub fn to_json(&self) -> Value {
        let checks: Vec<Value> = self.checks.iter().map(Check::to_json).collect();
        json!({
            "status": self.status().as_str(),
            "checks": checks,
        })
    }

    pub fn to_text(&self) -> String {
        let mut lines: Vec<String> = self.checks.iter().map(Check::to_text).collect();
        lines.push(format!("status: {}", self.status().as_str()));
        lines.join("\n")
    }
}
