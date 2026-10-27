use crate::stress_tally::Measure;
use crate::stress_tally::Tally;
use crate::stress_tally::LEGS;
use serde_json::json;
use serde_json::Map;
use serde_json::Value;
use std::collections::BTreeMap;
use std::time::Duration;

#[cfg(test)]
#[path = "stress_report_test.rs"]
mod stress_report_test;

const MIN_RESPONSE_PERCENT: u64 = 95;

/// The run's settings as the report states them.
pub struct Settings {
    pub mode: &'static str,
    pub measure: Measure,
    pub gateway: String,
    pub symbols: Vec<u32>,
    pub users: u32,
    pub connections: u32,
    pub target_rate: u32,
    pub duration_s: u64,
    pub count: u64,
    pub warmup: u64,
    pub timeout_ms: u64,
    pub is_crossing: bool,
}

/// What a finished run reports: the JSON object, and the correctness
/// failures it found (empty when the run is sound).
pub struct Report {
    pub json: Value,
    pub failures: Vec<String>,
}

/// Builds the report of a run from the tally of each symbol: counts, the
/// latency summary of the measured orders, and the server legs when the
/// run measures accepted orders.
///
/// Keys: `mode`, `status` (`passed`|`failed`), `failures`, `interrupted`,
/// `config`, the counters `offered` `submitted` `send_errors` `accepted`
/// `rejected` `timeouts` `errors` `completed` `fills` `filled_orders`, the
/// maps `rejected_by_reason` `errors_by_reason` `timestamp_errors`,
/// `elapsed_ms` (the send window and the drain), `latency_us` and
/// `send_lag_us` (microseconds),
/// `server_latency_ns` (one summary per leg) and `per_symbol`. A summary is
/// `n`, `min`, `p50`, `p95`, `p99`, `p99_9`, `max`.
pub fn build(
    settings: &Settings,
    mut per_symbol: BTreeMap<u32, Tally>,
    elapsed: Duration,
    is_interrupted: bool,
) -> Report {
    let mut total = Tally::default();
    for tally in per_symbol.values() {
        total.merge(tally);
    }
    let failures = check(settings, &total, &per_symbol);
    let mut symbols = Map::new();
    for (symbol_id, tally) in per_symbol.iter_mut() {
        symbols.insert(symbol_id.to_string(), symbol_json(tally));
    }
    let json = json!({
        "mode": settings.mode,
        "status": if failures.is_empty() { "passed" } else { "failed" },
        "failures": failures,
        "interrupted": is_interrupted,
        "config": config_json(settings),
        "offered": total.offered,
        "submitted": total.submitted,
        "send_errors": total.send_errors,
        "accepted": total.accepted,
        "rejected": total.rejected,
        "timeouts": total.timeouts,
        "errors": error_count(&total),
        "completed": total.completed(),
        "fills": total.fills,
        "filled_orders": total.filled_orders,
        "rejected_by_reason": total.rejected_by_reason,
        "errors_by_reason": total.errors_by_reason,
        "timestamp_errors": total.timestamp_errors,
        "elapsed_ms": elapsed.as_millis() as u64,
        "latency_us": total.latency_us.summary(),
        "send_lag_us": total.send_lag_us.summary(),
        "server_latency_ns": legs_json(&mut total),
        "per_symbol": symbols,
    });
    Report { json, failures }
}

fn config_json(settings: &Settings) -> Value {
    json!({
        "gateway": settings.gateway,
        "symbols": settings.symbols,
        "users": settings.users,
        "connections": settings.connections,
        "target_rate": settings.target_rate,
        "duration_s": settings.duration_s,
        "count": settings.count,
        "warmup": settings.warmup,
        "timeout_ms": settings.timeout_ms,
        "measures": match settings.measure {
            Measure::Accepted => "accepted",
            Measure::Rejected => "rejected",
        },
    })
}

fn legs_json(tally: &mut Tally) -> Value {
    let mut legs = Map::new();
    for (name, samples) in LEGS.iter().zip(tally.legs_ns.iter_mut()) {
        legs.insert((*name).to_string(), samples.summary());
    }
    Value::Object(legs)
}

fn symbol_json(tally: &mut Tally) -> Value {
    json!({
        "offered": tally.offered,
        "submitted": tally.submitted,
        "send_errors": tally.send_errors,
        "accepted": tally.accepted,
        "rejected": tally.rejected,
        "timeouts": tally.timeouts,
        "completed": tally.completed(),
        "fills": tally.fills,
        "filled_orders": tally.filled_orders,
        "latency_us": tally.latency_us.summary(),
        "server_latency_ns": legs_json(tally),
    })
}

fn error_count(tally: &Tally) -> u64 {
    tally.errors_by_reason.values().sum()
}

fn check(settings: &Settings, total: &Tally, per_symbol: &BTreeMap<u32, Tally>) -> Vec<String> {
    let mut failures = Vec::new();
    if total.latency_us.is_empty() {
        failures.push(match settings.measure {
            Measure::Accepted => "zero accepted latency samples".to_string(),
            Measure::Rejected => "zero rejected latency samples".to_string(),
        });
    }
    if total.offered != total.submitted + total.send_errors {
        failures.push("offered accounting does not close".to_string());
    }
    if total.submitted != total.completed() + total.timeouts {
        failures.push("order accounting does not close".to_string());
    }
    if error_count(total) > 0 {
        failures.push(format!(
            "protocol errors: {}",
            describe_counts(&total.errors_by_reason)
        ));
    }
    if total.completed() * 100 < MIN_RESPONSE_PERCENT * total.submitted {
        failures.push(format!(
            "answered orders below {MIN_RESPONSE_PERCENT}%: {} of {}",
            total.completed(),
            total.submitted
        ));
    }
    match settings.measure {
        Measure::Accepted => check_accepted(settings, total, per_symbol, &mut failures),
        Measure::Rejected => {
            if total.accepted > 0 {
                failures.push(format!("{} orders were accepted", total.accepted));
            }
        }
    }
    failures
}

fn check_accepted(
    settings: &Settings,
    total: &Tally,
    per_symbol: &BTreeMap<u32, Tally>,
    failures: &mut Vec<String>,
) {
    if settings.is_crossing && total.filled_orders != total.accepted {
        failures.push("accepted crossing orders did not all fill".to_string());
    }
    if !total.timestamp_errors.is_empty() {
        failures.push(format!(
            "invalid server timestamps: {}",
            describe_counts(&total.timestamp_errors)
        ));
    }
    if settings.is_crossing {
        check_legs(total, "", failures);
        for (symbol_id, tally) in per_symbol {
            check_legs(tally, &format!("symbol {symbol_id} "), failures);
        }
    }
    for symbol_id in &settings.symbols {
        let completed = per_symbol
            .get(symbol_id)
            .map_or(0, |tally| tally.completed());
        if completed == 0 {
            failures.push(format!("symbol {symbol_id} completed no work"));
        }
    }
}

fn check_legs(tally: &Tally, label: &str, failures: &mut Vec<String>) {
    for (name, samples) in LEGS.iter().zip(&tally.legs_ns) {
        let expected = match *name {
            "risk_to_me" | "matching" => tally.filled_orders,
            _ => tally.accepted,
        };
        if samples.len() != expected {
            failures.push(format!("{label}{name} samples do not equal {expected}"));
        }
    }
}

fn describe_counts(counts: &BTreeMap<String, u64>) -> String {
    counts
        .iter()
        .map(|(reason, count)| format!("{reason}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}
