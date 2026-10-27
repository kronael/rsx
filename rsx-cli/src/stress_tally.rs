use crate::outcome::failure_name;
use rsx_messages::WsFrame;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::time::Instant;

#[cfg(test)]
#[path = "stress_tally_test.rs"]
mod stress_tally_test;

/// The server-side legs of an order, in the order `Stamps` lists the clocks
/// they are cut from; the report names each leg.
pub const LEGS: [&str; 5] = [
    "internal",
    "gateway_to_risk",
    "risk_to_me",
    "matching",
    "match_done_to_gateway",
];

const STATUS_FAILED: u8 = 3;

type Stamps = [u64; 5];

/// Which orders contribute latency samples: those the exchange accepted, or
/// those it rejected (a run that expects the reject).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Measure {
    Accepted,
    Rejected,
}

/// Latency samples of one kind; the summary sorts them in place.
#[derive(Clone, Debug, Default)]
pub struct Samples(Vec<u64>);

impl Samples {
    pub fn push(&mut self, value: u64) {
        self.0.push(value);
    }

    pub fn extend(&mut self, other: &Samples) {
        self.0.extend_from_slice(&other.0);
    }

    pub fn len(&self) -> u64 {
        self.0.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `n`, `min`, `max` and the nearest-rank `p50`, `p95`, `p99`, `p99_9`
    /// of the samples; every value but `n` is null when there are none.
    pub fn summary(&mut self) -> Value {
        self.0.sort_unstable();
        json!({
            "n": self.len(),
            "min": self.0.first(),
            "p50": percentile(&self.0, 50.0),
            "p95": percentile(&self.0, 95.0),
            "p99": percentile(&self.0, 99.0),
            "p99_9": percentile(&self.0, 99.9),
            "max": self.0.last(),
        })
    }
}

fn percentile(values: &[u64], percent: f64) -> Option<u64> {
    let count = values.len();
    if count == 0 {
        return None;
    }
    let rank = ((percent / 100.0) * count as f64).ceil() as usize;
    values.get(rank.clamp(1, count) - 1).copied()
}

/// Everything one run (or one symbol of it) counted.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    pub offered: u64,
    pub submitted: u64,
    pub send_errors: u64,
    pub accepted: u64,
    pub rejected: u64,
    pub timeouts: u64,
    pub fills: u64,
    pub filled_orders: u64,
    pub rejected_by_reason: BTreeMap<String, u64>,
    pub errors_by_reason: BTreeMap<String, u64>,
    pub timestamp_errors: BTreeMap<String, u64>,
    pub latency_us: Samples,
    pub send_lag_us: Samples,
    pub legs_ns: [Samples; 5],
}

impl Tally {
    /// Orders that got an answer: accepted or rejected.
    pub fn completed(&self) -> u64 {
        self.accepted + self.rejected
    }

    pub fn merge(&mut self, other: &Tally) {
        self.offered += other.offered;
        self.submitted += other.submitted;
        self.send_errors += other.send_errors;
        self.accepted += other.accepted;
        self.rejected += other.rejected;
        self.timeouts += other.timeouts;
        self.fills += other.fills;
        self.filled_orders += other.filled_orders;
        merge_counts(&mut self.rejected_by_reason, &other.rejected_by_reason);
        merge_counts(&mut self.errors_by_reason, &other.errors_by_reason);
        merge_counts(&mut self.timestamp_errors, &other.timestamp_errors);
        self.latency_us.extend(&other.latency_us);
        self.send_lag_us.extend(&other.send_lag_us);
        for (samples, theirs) in self.legs_ns.iter_mut().zip(&other.legs_ns) {
            samples.extend(theirs);
        }
    }

    pub fn count_error(&mut self, reason: &str) {
        *self.errors_by_reason.entry(reason.to_string()).or_default() += 1;
    }

    fn count_rejection(&mut self, reason: String) {
        self.rejected += 1;
        *self.rejected_by_reason.entry(reason).or_default() += 1;
    }

    fn count_timestamp_error(&mut self, reason: &str) {
        *self.timestamp_errors.entry(reason.to_string()).or_default() += 1;
    }
}

fn merge_counts(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (reason, count) in from {
        *into.entry(reason.clone()).or_default() += count;
    }
}

struct Live {
    intended: Instant,
    is_filled: bool,
    fill_stamps: Stamps,
}

/// Matches the gateway's replies on one connection to the orders sent on it.
///
/// A reply does not echo the client order id, so the first frame that names a
/// new order id binds it to the oldest order still unanswered; the connection
/// carries one symbol, so the matching engine answers in send order. Every
/// stress order is IOC, so the terminal update is the last frame of an order.
/// Latency runs from the order's intended send time, not from when the send
/// happened, so a client that falls behind its schedule shows the queueing.
pub struct Tracker {
    measure: Measure,
    awaiting: VecDeque<Instant>,
    live: HashMap<String, Live>,
    pub tally: Tally,
}

impl Tracker {
    pub fn new(measure: Measure) -> Self {
        Self {
            measure,
            awaiting: VecDeque::new(),
            live: HashMap::new(),
            tally: Tally::default(),
        }
    }

    /// Orders sent and not yet answered.
    pub fn is_idle(&self) -> bool {
        self.awaiting.is_empty() && self.live.is_empty()
    }

    /// An order scheduled for `intended` went out at `now`.
    pub fn record_sent(&mut self, intended: Instant, now: Instant) {
        self.tally.offered += 1;
        self.tally.submitted += 1;
        self.awaiting.push_back(intended);
        self.tally
            .send_lag_us
            .push(micros(now.saturating_duration_since(intended)));
    }

    /// An order was due but could not be sent.
    pub fn record_send_failure(&mut self) {
        self.tally.offered += 1;
        self.tally.send_errors += 1;
    }

    /// Gives up on unbound orders sent more than `timeout` before `now`, so a
    /// lost order does not shift the binding of every later reply.
    pub fn expire(&mut self, now: Instant, timeout: Duration) {
        while let Some(intended) = self.awaiting.front() {
            if now.saturating_duration_since(*intended) < timeout {
                break;
            }
            self.awaiting.pop_front();
            self.tally.timeouts += 1;
        }
    }

    /// Counts every order still unanswered as a timeout.
    pub fn finish(&mut self) {
        self.tally.timeouts += (self.awaiting.len() + self.live.len()) as u64;
        self.awaiting.clear();
        self.live.clear();
    }

    /// Accounts for one gateway frame received at `now`.
    pub fn accept(&mut self, frame: &WsFrame, now: Instant) {
        match frame {
            WsFrame::Error { code, .. } => {
                let Some(intended) = self.awaiting.pop_front() else {
                    self.tally.count_error("unbound_error");
                    return;
                };
                self.tally.count_rejection(format!("E{code}"));
                self.record_latency(Measure::Rejected, intended, now);
            }
            WsFrame::Fill {
                taker_order_id,
                gw_in_ns,
                risk_in_ns,
                me_in_ns,
                match_done_ns,
                gw_out_ns,
                ..
            } => {
                let Some(live) = bind(&mut self.live, &mut self.awaiting, taker_order_id) else {
                    return;
                };
                self.tally.fills += 1;
                if !live.is_filled {
                    live.is_filled = true;
                    self.tally.filled_orders += 1;
                }
                live.fill_stamps = [
                    *gw_in_ns,
                    *risk_in_ns,
                    *me_in_ns,
                    *match_done_ns,
                    *gw_out_ns,
                ];
            }
            WsFrame::OrderUpdate {
                order_id,
                status,
                reason,
                gw_in_ns,
                risk_in_ns,
                me_in_ns,
                match_done_ns,
                gw_out_ns,
                ..
            } => {
                let Some(live) = take(&mut self.live, &mut self.awaiting, order_id) else {
                    return;
                };
                if *status == STATUS_FAILED {
                    self.tally.count_rejection(failure_name(*reason));
                    self.record_latency(Measure::Rejected, live.intended, now);
                    return;
                }
                self.tally.accepted += 1;
                self.record_latency(Measure::Accepted, live.intended, now);
                let update = [
                    *gw_in_ns,
                    *risk_in_ns,
                    *me_in_ns,
                    *match_done_ns,
                    *gw_out_ns,
                ];
                if self.measure == Measure::Accepted {
                    self.record_legs(&live, update);
                }
            }
            _ => self.tally.count_error("unknown_frame"),
        }
    }

    fn record_latency(&mut self, outcome: Measure, intended: Instant, now: Instant) {
        if outcome == self.measure {
            self.tally
                .latency_us
                .push(micros(now.saturating_duration_since(intended)));
        }
    }

    fn record_legs(&mut self, live: &Live, update: Stamps) {
        let mut stamps = [0; 5];
        for ((stamp, from_update), from_fill) in stamps.iter_mut().zip(update).zip(live.fill_stamps)
        {
            *stamp = if from_update != 0 {
                from_update
            } else {
                from_fill
            };
        }
        let [gw_in, risk_in, me_in, match_done, gw_out] = stamps;
        if gw_in == 0 || risk_in == 0 || match_done == 0 || gw_out == 0 {
            self.tally
                .count_timestamp_error("missing_completion_timestamp");
            return;
        }
        if !(gw_in <= risk_in && risk_in <= match_done && match_done <= gw_out) {
            self.tally
                .count_timestamp_error("non_monotone_completion_timestamp");
            return;
        }
        if live.is_filled {
            if me_in == 0 {
                self.tally
                    .count_timestamp_error("missing_matching_timestamp");
                return;
            }
            if !(risk_in <= me_in && me_in <= match_done) {
                self.tally
                    .count_timestamp_error("non_monotone_matching_timestamp");
                return;
            }
        }
        let [internal, to_risk, to_engine, matching, to_gateway] = &mut self.tally.legs_ns;
        internal.push(gw_out - gw_in);
        to_risk.push(risk_in - gw_in);
        to_gateway.push(gw_out - match_done);
        if live.is_filled {
            to_engine.push(me_in - risk_in);
            matching.push(match_done - me_in);
        }
    }
}

fn bind<'a>(
    live: &'a mut HashMap<String, Live>,
    awaiting: &mut VecDeque<Instant>,
    order_id: &str,
) -> Option<&'a mut Live> {
    if !live.contains_key(order_id) {
        let intended = awaiting.pop_front()?;
        live.insert(
            order_id.to_string(),
            Live {
                intended,
                is_filled: false,
                fill_stamps: [0; 5],
            },
        );
    }
    live.get_mut(order_id)
}

fn take(
    live: &mut HashMap<String, Live>,
    awaiting: &mut VecDeque<Instant>,
    order_id: &str,
) -> Option<Live> {
    bind(live, awaiting, order_id)?;
    live.remove(order_id)
}

fn micros(elapsed: Duration) -> u64 {
    elapsed.as_micros() as u64
}
