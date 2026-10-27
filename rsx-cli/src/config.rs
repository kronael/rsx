use crate::database;
use crate::output::Failure;
use crate::output::Outcome;
use crate::runtime_utils::block_on;
use rsx_risk::config::schedule_symbol_config;
use rsx_risk::config::ConfigField;
use rsx_risk::config::ScheduledVersion;
use rsx_types::time_utils::time_ms;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::ErrorKind;

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;

/// One symbol as `config show` prints it: every scheduled version and the
/// row the matching engine applied last.
#[derive(Debug, PartialEq)]
pub struct SymbolView {
    pub symbol_id: u32,
    pub name: Option<String>,
    pub scheduled: Vec<Value>,
    pub applied: Option<Value>,
}

impl SymbolView {
    /// The version risk runs on at `now_ms`: the highest version whose
    /// `effective_at_ms` has passed.
    pub fn active_version(&self, now_ms: i64) -> Option<i64> {
        self.scheduled
            .iter()
            .filter(|row| {
                row["effective_at_ms"]
                    .as_i64()
                    .is_some_and(|at| at <= now_ms)
            })
            .filter_map(|row| row["config_version"].as_i64())
            .max()
    }

    /// `{"symbol_id","name","active_version","scheduled":[row],"applied":row|null}`;
    /// a row has the columns of its table.
    pub fn to_json(&self, now_ms: i64) -> Value {
        json!({
            "symbol_id": self.symbol_id,
            "name": self.name,
            "active_version": self.active_version(now_ms),
            "scheduled": self.scheduled,
            "applied": self.applied,
        })
    }

    pub fn to_text(&self, now_ms: i64) -> String {
        let version = |label: &str, value: Option<i64>| match value {
            Some(version) => format!("{label} v{version}"),
            None => format!("{label} -"),
        };
        let applied = self
            .applied
            .as_ref()
            .and_then(|row| row["config_version"].as_i64());
        let mut text = format!(
            "symbol {} {}  {}  {}\n",
            self.symbol_id,
            self.name.as_deref().unwrap_or("-"),
            version("active", self.active_version(now_ms)),
            version("applied", applied),
        );
        text.push_str(
            "  version  effective_at_ms  tick_size  lot_size  im_bps  mm_bps  \
             max_leverage  taker_bps  maker_bps  min_notional\n",
        );
        for row in &self.scheduled {
            let cell = |column: &str| match &row[column] {
                Value::Null => "-".to_string(),
                value => value.to_string(),
            };
            text.push_str(&format!(
                "  {:>7}  {:>15}  {:>9}  {:>8}  {:>6}  {:>6}  {:>12}  {:>9}  {:>9}  {}\n",
                cell("config_version"),
                cell("effective_at_ms"),
                cell("tick_size"),
                cell("lot_size"),
                cell("initial_margin_rate_bps"),
                cell("maintenance_margin_rate_bps"),
                cell("max_leverage"),
                cell("taker_fee_bps"),
                cell("maker_fee_bps"),
                cell("min_notional"),
            ));
        }
        text
    }
}

/// `{"symbol_id","config_version","effective_at_ms"}` of a scheduled version.
pub fn scheduled_json(scheduled: &ScheduledVersion) -> Value {
    json!({
        "symbol_id": scheduled.symbol_id,
        "config_version": scheduled.config_version,
        "effective_at_ms": scheduled.effective_at_ms,
    })
}

/// A `--set column=value` setting: the column of `symbol_config_schedule`
/// and a whole number.
pub fn parse_setting(text: &str) -> Result<(ConfigField, i64), String> {
    let Some((column, value)) = text.split_once('=') else {
        return Err(format!("{text:?} is not column=value"));
    };
    let Some(field) = ConfigField::from_column(column) else {
        let known: Vec<&str> = ConfigField::ALL
            .iter()
            .map(|field| field.column())
            .collect();
        return Err(format!(
            "{column:?} is not a setting (settings: {})",
            known.join(", ")
        ));
    };
    let value = value
        .parse::<i64>()
        .map_err(|_| format!("{value:?} is not a whole number"))?;
    Ok((field, value))
}

/// A writer error as the failure the CLI reports: a rejected change is
/// `Misuse`, everything else `Runtime`.
pub fn to_failure(error: std::io::Error) -> Failure {
    match error.kind() {
        ErrorKind::InvalidInput => Failure::misuse(error),
        _ => Failure::runtime(error),
    }
}

fn view_of(views: &mut BTreeMap<u32, SymbolView>, symbol_id: i32) -> &mut SymbolView {
    views.entry(symbol_id as u32).or_insert(SymbolView {
        symbol_id: symbol_id as u32,
        name: None,
        scheduled: Vec::new(),
        applied: None,
    })
}

async fn load_views(url: &str, only: Option<u32>) -> Result<Vec<SymbolView>, Failure> {
    let client = database::open(url).await?;
    let read = |error: tokio_postgres::Error| {
        Failure::runtime(format!("failed to read the symbol config: {error}"))
    };
    let parse = |text: String| {
        serde_json::from_str::<Value>(&text)
            .map_err(|error| Failure::runtime(format!("bad config row: {error}")))
    };
    let mut views: BTreeMap<u32, SymbolView> = BTreeMap::new();
    let names = client
        .query("SELECT symbol_id, symbol_name FROM symbol_static", &[])
        .await
        .map_err(read)?;
    let scheduled = client
        .query(
            "SELECT symbol_id, row_to_json(c)::text FROM symbol_config_schedule c \
             ORDER BY symbol_id, config_version",
            &[],
        )
        .await
        .map_err(read)?;
    let applied = client
        .query(
            "SELECT symbol_id, row_to_json(a)::text FROM symbol_config_applied a",
            &[],
        )
        .await
        .map_err(read)?;
    for row in &names {
        view_of(&mut views, row.get(0)).name = Some(row.get(1));
    }
    for row in &scheduled {
        let parsed = parse(row.get(1))?;
        view_of(&mut views, row.get(0)).scheduled.push(parsed);
    }
    for row in &applied {
        let parsed = parse(row.get(1))?;
        view_of(&mut views, row.get(0)).applied = Some(parsed);
    }
    let views: Vec<SymbolView> = views
        .into_values()
        .filter(|entry| only.is_none_or(|symbol_id| entry.symbol_id == symbol_id))
        .collect();
    if let (Some(symbol_id), true) = (only, views.is_empty()) {
        return Err(Failure::runtime(format!(
            "symbol {symbol_id} has no config"
        )));
    }
    Ok(views)
}

/// Prints every symbol's scheduled versions and applied row, or only
/// `only`'s: `{"symbols":[SymbolView::to_json]}` under `--json`.
pub fn show(url: &str, only: Option<u32>, is_json: bool) -> Outcome {
    let now_ms = time_ms() as i64;
    let views = block_on(load_views(url, only))?;
    if is_json {
        let symbols: Vec<Value> = views.iter().map(|view| view.to_json(now_ms)).collect();
        println!("{}", json!({ "symbols": symbols }));
    } else {
        for view in &views {
            print!("{}", view.to_text(now_ms));
        }
    }
    Ok(())
}

async fn do_schedule(
    url: &str,
    symbol_id: u32,
    changes: &[(ConfigField, i64)],
    effective_at_ms: u64,
) -> Result<ScheduledVersion, Failure> {
    let mut client = database::open(url).await?;
    schedule_symbol_config(&mut client, symbol_id, changes, effective_at_ms)
        .await
        .map_err(to_failure)
}

/// Inserts a new version of `symbol_id`'s config with `changes` applied,
/// effective at `effective_at_ms` (default: now). The active row is not
/// touched: risk and the matching engine apply the new one on their next
/// poll once it is effective.
pub fn schedule(
    url: &str,
    symbol_id: u32,
    changes: &[(ConfigField, i64)],
    effective_at_ms: Option<u64>,
    is_json: bool,
) -> Outcome {
    let effective_at_ms = effective_at_ms.unwrap_or_else(time_ms);
    let scheduled = block_on(do_schedule(url, symbol_id, changes, effective_at_ms))?;
    if is_json {
        println!("{}", scheduled_json(&scheduled));
    } else {
        println!(
            "scheduled v{} for symbol {}, effective at {} ms",
            scheduled.config_version, scheduled.symbol_id, scheduled.effective_at_ms
        );
    }
    Ok(())
}
