use rsx_messages::WsFrame;
use serde_json::json;
use serde_json::Value;

#[cfg(test)]
#[path = "outcome_test.rs"]
mod outcome_test;

const FAILURE_REASONS: [&str; 15] = [
    "INVALID_TICK_SIZE",
    "INVALID_LOT_SIZE",
    "SYMBOL_NOT_FOUND",
    "DUPLICATE_ORDER_ID",
    "INSUFFICIENT_MARGIN",
    "OVERLOADED",
    "INTERNAL_ERROR",
    "REDUCE_ONLY_VIOLATION",
    "NETWORK_ERROR",
    "RATE_LIMIT",
    "TIMEOUT",
    "USER_IN_LIQUIDATION",
    "WRONG_SHARD",
    "FILL_OR_KILL",
    "ORDER_TOO_SMALL",
];

const CANCEL_REASONS: [&str; 6] = [
    "USER_CANCEL",
    "REDUCE_ONLY",
    "EXPIRY",
    "SYSTEM",
    "POST_ONLY_REJECT",
    "OTHER",
];

const STATUS_FILLED: u8 = 0;
const STATUS_RESTING: u8 = 1;
const STATUS_CANCELLED: u8 = 2;
const STATUS_FAILED: u8 = 3;
const REASON_TIMEOUT: u8 = 10;

/// How an order ended, as the CLI reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Filled,
    /// Some quantity filled and the rest rests or was cancelled.
    Partial,
    /// Rests on the book with no fill.
    Resting,
    /// Cancelled with no fill, for the reason given.
    Cancelled,
    Rejected,
    /// No answer in time, or the gateway's own TIMEOUT: the order's fate is
    /// unknown.
    Timeout,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Filled => "filled",
            Status::Partial => "partial",
            Status::Resting => "resting",
            Status::Cancelled => "cancelled",
            Status::Rejected => "rejected",
            Status::Timeout => "timeout",
        }
    }

    /// True for the statuses that make the command exit 1.
    pub fn is_failure(self) -> bool {
        matches!(self, Status::Rejected | Status::Timeout)
    }
}

#[derive(Debug, PartialEq)]
pub struct Fill {
    pub price: i64,
    pub qty: i64,
    pub timestamp_ns: u64,
}

/// The terminal result of one order or cancel. `reply` is the kind letter of
/// the gateway frame that ended it (`U` or `E`), `None` for a local timeout.
/// With `E`, `reason_code` is the error code and `reason` its message. With a
/// `U`, `reason` is the `FailureReason` name (specs/2/18-messages.md) of a
/// status-3 update, or the `CANCEL_REASON_*` name of a status-2 update.
#[derive(Debug)]
pub struct Verdict {
    pub status: Status,
    pub reason_code: Option<u32>,
    pub reason: Option<String>,
    pub reply: Option<char>,
    pub order_id: Option<String>,
    pub fills: Vec<Fill>,
    pub frames: Vec<Value>,
}

/// The name of a `FailureReason` value.
pub fn failure_name(code: u8) -> String {
    match FAILURE_REASONS.get(usize::from(code)) {
        Some(name) => (*name).to_string(),
        None => format!("UNKNOWN_{code}"),
    }
}

/// The name of a `CANCEL_REASON_*` value.
pub fn cancel_name(code: u8) -> String {
    match CANCEL_REASONS.get(usize::from(code)) {
        Some(name) => (*name).to_string(),
        None => format!("UNKNOWN_{code}"),
    }
}

/// Collects the frames that answer one new order on a connection that has no
/// other order in flight. The first `U` or `F` names the order; frames of
/// other orders of the same user are skipped after that.
#[derive(Default)]
pub struct PlaceTracker {
    order_id: Option<String>,
    fills: Vec<Fill>,
    frames: Vec<Value>,
}

impl PlaceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes one gateway frame (not a heartbeat) and its raw JSON; returns the
    /// verdict when the frame ends the order, else `None`.
    pub fn accept(&mut self, frame: &WsFrame, raw: Value) -> Option<Verdict> {
        match frame {
            WsFrame::Error { code, message, .. } => {
                self.frames.push(raw);
                Some(self.finish(
                    Status::Rejected,
                    Some(*code),
                    Some(message.clone()),
                    Some('E'),
                ))
            }
            WsFrame::OrderUpdate {
                order_id,
                status,
                filled_qty,
                reason,
                ..
            } => {
                if !self.pairs(order_id) {
                    return None;
                }
                self.frames.push(raw);
                match *status {
                    STATUS_FILLED => Some(self.finish(Status::Filled, None, None, Some('U'))),
                    STATUS_RESTING => {
                        let status = if self.fills.is_empty() {
                            Status::Resting
                        } else {
                            Status::Partial
                        };
                        Some(self.finish(status, None, None, Some('U')))
                    }
                    STATUS_CANCELLED => {
                        let status = if self.fills.is_empty() && *filled_qty == 0 {
                            Status::Cancelled
                        } else {
                            Status::Partial
                        };
                        let (code, name) = cancel_reason(*reason);
                        Some(self.finish(status, code, name, Some('U')))
                    }
                    STATUS_FAILED => Some(self.fail(*reason)),
                    _ => None,
                }
            }
            WsFrame::Fill {
                taker_order_id,
                price,
                qty,
                timestamp_ns,
                ..
            } => {
                if self.pairs(taker_order_id) {
                    self.frames.push(raw);
                    self.fills.push(Fill {
                        price: *price,
                        qty: *qty,
                        timestamp_ns: *timestamp_ns,
                    });
                }
                None
            }
            _ => None,
        }
    }

    /// The verdict when no frame ended the order in time.
    pub fn expire(&mut self) -> Verdict {
        self.finish(Status::Timeout, None, None, None)
    }

    fn pairs(&mut self, order_id: &str) -> bool {
        match &self.order_id {
            Some(own) => own == order_id,
            None => {
                self.order_id = Some(order_id.to_string());
                true
            }
        }
    }

    fn fail(&mut self, reason: u8) -> Verdict {
        let status = if reason == REASON_TIMEOUT {
            Status::Timeout
        } else {
            Status::Rejected
        };
        self.finish(
            status,
            Some(u32::from(reason)),
            Some(failure_name(reason)),
            Some('U'),
        )
    }

    fn finish(
        &mut self,
        status: Status,
        reason_code: Option<u32>,
        reason: Option<String>,
        reply: Option<char>,
    ) -> Verdict {
        Verdict {
            status,
            reason_code,
            reason,
            reply,
            order_id: self.order_id.take(),
            fills: std::mem::take(&mut self.fills),
            frames: std::mem::take(&mut self.frames),
        }
    }
}

/// Collects the frames that answer one cancel. With `order_id` known only
/// that order's updates count; with a cid the first status-2 or status-3
/// update does.
pub struct CancelTracker {
    order_id: Option<String>,
    frames: Vec<Value>,
}

impl CancelTracker {
    pub fn new(order_id: Option<String>) -> Self {
        Self {
            order_id,
            frames: Vec::new(),
        }
    }

    /// Takes one gateway frame (not a heartbeat) and its raw JSON; returns the
    /// verdict when the frame ends the cancel, else `None`.
    pub fn accept(&mut self, frame: &WsFrame, raw: Value) -> Option<Verdict> {
        match frame {
            WsFrame::Error { code, message, .. } => {
                self.frames.push(raw);
                Some(self.finish(
                    Status::Rejected,
                    Some(*code),
                    Some(message.clone()),
                    Some('E'),
                    None,
                ))
            }
            WsFrame::OrderUpdate {
                order_id,
                status,
                reason,
                ..
            } => {
                if self.order_id.as_ref().is_some_and(|own| own != order_id) {
                    return None;
                }
                match *status {
                    STATUS_CANCELLED => {
                        self.frames.push(raw);
                        let (code, name) = cancel_reason(*reason);
                        Some(self.finish(
                            Status::Cancelled,
                            code,
                            name,
                            Some('U'),
                            Some(order_id.clone()),
                        ))
                    }
                    STATUS_FAILED => {
                        self.frames.push(raw);
                        let status = if *reason == REASON_TIMEOUT {
                            Status::Timeout
                        } else {
                            Status::Rejected
                        };
                        Some(self.finish(
                            status,
                            Some(u32::from(*reason)),
                            Some(failure_name(*reason)),
                            Some('U'),
                            Some(order_id.clone()),
                        ))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The verdict when no frame ended the cancel in time.
    pub fn expire(&mut self) -> Verdict {
        self.finish(Status::Timeout, None, None, None, self.order_id.clone())
    }

    fn finish(
        &mut self,
        status: Status,
        reason_code: Option<u32>,
        reason: Option<String>,
        reply: Option<char>,
        order_id: Option<String>,
    ) -> Verdict {
        Verdict {
            status,
            reason_code,
            reason,
            reply,
            order_id,
            fills: Vec::new(),
            frames: std::mem::take(&mut self.frames),
        }
    }
}

fn cancel_reason(reason: u8) -> (Option<u32>, Option<String>) {
    (Some(u32::from(reason)), Some(cancel_name(reason)))
}

impl Verdict {
    /// The `order place` object: `cid`, `status`, `reason_code`, `reason`,
    /// `reply`, `order_id`, `fills` (`px`, `qty`, `ts_ns`), `frames` (the
    /// order's gateway frames as received, heartbeats excluded) and `rtt_us`
    /// (send to the ending frame, or to the timeout).
    pub fn place_json(&self, cid: &str, rtt_us: u64) -> Value {
        let fills: Vec<Value> = self
            .fills
            .iter()
            .map(|fill| {
                json!({
                    "px": fill.price,
                    "qty": fill.qty,
                    "ts_ns": fill.timestamp_ns,
                })
            })
            .collect();
        json!({
            "cid": cid,
            "status": self.status.as_str(),
            "reason_code": self.reason_code,
            "reason": self.reason,
            "reply": self.reply,
            "order_id": self.order_id,
            "fills": fills,
            "frames": self.frames,
            "rtt_us": rtt_us,
        })
    }

    /// The `order cancel` object: the `order place` keys without `fills`;
    /// `cid` is null when the cancel named an order id.
    pub fn cancel_json(&self, cid: Option<&str>, rtt_us: u64) -> Value {
        json!({
            "cid": cid,
            "status": self.status.as_str(),
            "reason_code": self.reason_code,
            "reason": self.reason,
            "reply": self.reply,
            "order_id": self.order_id,
            "frames": self.frames,
            "rtt_us": rtt_us,
        })
    }

    /// One text line: status, the key of the order, the fill count when there
    /// are fills, and the reason.
    pub fn to_text(&self, key: &str, rtt_us: u64) -> String {
        let mut line = format!("{} {key}", self.status.as_str());
        if let Some(order_id) = &self.order_id {
            line.push_str(&format!(" order_id={order_id}"));
        }
        if !self.fills.is_empty() {
            let filled: i64 = self.fills.iter().map(|fill| fill.qty).sum();
            line.push_str(&format!(" fills={} filled_qty={filled}", self.fills.len()));
        }
        if let (Some(code), Some(reason)) = (self.reason_code, &self.reason) {
            if self.reply == Some('E') {
                line.push_str(&format!(" error={code} ({reason})"));
            } else {
                line.push_str(&format!(" reason={reason}({code})"));
            }
        }
        line.push_str(&format!(" rtt_us={rtt_us}"));
        line
    }
}
