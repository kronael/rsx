use crate::database;
use crate::output::Failure;
use crate::output::Outcome;
use crate::pg::position_from_row;
use crate::pg::POSITION_COLUMNS;
use crate::runtime_utils::block_on;
use rsx_risk::Position;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio_postgres::Client;

#[cfg(test)]
#[path = "account_test.rs"]
mod account_test;

/// Test accounts `account seed` creates by default: users 1 to 1001.
pub const DEFAULT_SEED_ACCOUNTS: u32 = 1001;

/// 1e17 raw units: it must exceed the initial margin of the maker's largest
/// resting orders.
pub const DEFAULT_SEED_COLLATERAL: i64 = 100_000_000_000_000_000;

const LIQUIDATIONS_SHOWN: i64 = 50;

/// Mark prices by symbol, raw, for the unrealized PnL of a position.
pub type Marks = BTreeMap<u32, i64>;

/// A `--mark symbol=price` setting: a symbol id and a raw price.
pub fn parse_mark(text: &str) -> Result<(u32, i64), String> {
    let Some((symbol_id, price)) = text.split_once('=') else {
        return Err(format!("{text:?} is not symbol=price"));
    };
    let symbol_id = symbol_id
        .parse::<u32>()
        .map_err(|_| format!("{symbol_id:?} is not a symbol id"))?;
    let price = price
        .parse::<i64>()
        .map_err(|_| format!("{price:?} is not a raw price"))?;
    Ok((symbol_id, price))
}

/// Margin risk holds for one open order.
#[derive(Debug, PartialEq)]
pub struct Frozen {
    pub order_id: String,
    pub symbol_id: u32,
    pub amount: i64,
}

/// An account as risk persisted it: collateral, the open positions and the
/// margin frozen per open order.
#[derive(Debug)]
pub struct Account {
    pub user_id: u32,
    pub collateral: i64,
    pub positions: Vec<Position>,
    pub frozen: Vec<Frozen>,
}

impl Account {
    pub fn frozen_total(&self) -> Result<i64, Failure> {
        self.frozen
            .iter()
            .try_fold(0i64, |total, frozen| total.checked_add(frozen.amount))
            .ok_or_else(|| {
                Failure::runtime(format!("user {}: frozen margin overflows", self.user_id))
            })
    }

    /// `{"user_id","collateral","positions":[{"symbol_id","net_qty","avg_px",
    /// "upnl"}],"frozen":[{"order_id","symbol_id","amount"}]}`. `upnl` is
    /// null for a symbol without a mark.
    pub fn to_json(&self, marks: &Marks) -> Result<Value, Failure> {
        let mut positions = Vec::with_capacity(self.positions.len());
        for position in &self.positions {
            let upnl = match marks.get(&position.symbol_id) {
                Some(mark) => Some(position.unrealized_pnl(*mark).map_err(|error| {
                    Failure::runtime(format!(
                        "user {} symbol {}: {error}",
                        self.user_id, position.symbol_id
                    ))
                })?),
                None => None,
            };
            positions.push(json!({
                "symbol_id": position.symbol_id,
                "net_qty": position.net_qty(),
                "avg_px": position.avg_entry(),
                "upnl": upnl,
            }));
        }
        let frozen: Vec<Value> = self
            .frozen
            .iter()
            .map(|frozen| {
                json!({
                    "order_id": frozen.order_id,
                    "symbol_id": frozen.symbol_id,
                    "amount": frozen.amount,
                })
            })
            .collect();
        Ok(json!({
            "user_id": self.user_id,
            "collateral": self.collateral,
            "positions": positions,
            "frozen": frozen,
        }))
    }

    pub fn to_text(&self, marks: &Marks) -> Result<String, Failure> {
        let mut text = format!(
            "user {}  collateral {}  frozen {} ({} orders)\n",
            self.user_id,
            self.collateral,
            self.frozen_total()?,
            self.frozen.len()
        );
        for position in &self.positions {
            let upnl = match marks.get(&position.symbol_id) {
                Some(mark) => position
                    .unrealized_pnl(*mark)
                    .map_or("overflow".to_string(), |upnl| upnl.to_string()),
                None => "-".to_string(),
            };
            text.push_str(&format!(
                "  symbol {}  net_qty {}  avg_px {}  upnl {}\n",
                position.symbol_id,
                position.net_qty(),
                position.avg_entry(),
                upnl
            ));
        }
        Ok(text)
    }
}

fn read_failure(error: impl std::fmt::Display) -> Failure {
    Failure::runtime(format!("failed to read risk's accounts: {error}"))
}

/// Every account, or `only`'s, with the positions that are open and the
/// frozen orders. Rows of a user without an `accounts` row are not listed.
async fn load_accounts(client: &Client, only: Option<u32>) -> Result<Vec<Account>, Failure> {
    let filter = only.map(|user_id| user_id as i32);
    let accounts = client
        .query(
            "SELECT user_id, collateral FROM accounts \
             WHERE ($1::int IS NULL OR user_id = $1) ORDER BY user_id",
            &[&filter],
        )
        .await
        .map_err(read_failure)?;
    let positions = client
        .query(
            &format!(
                "SELECT {POSITION_COLUMNS} FROM positions \
                 WHERE long_qty <> short_qty AND ($1::int IS NULL OR user_id = $1) \
                 ORDER BY user_id, symbol_id"
            ),
            &[&filter],
        )
        .await
        .map_err(read_failure)?;
    let frozen = client
        .query(
            "SELECT user_id, order_id_hi, order_id_lo, symbol_id, amount \
             FROM frozen_orders WHERE ($1::int IS NULL OR user_id = $1) \
             ORDER BY user_id, order_id_hi, order_id_lo",
            &[&filter],
        )
        .await
        .map_err(read_failure)?;
    let mut by_user: BTreeMap<u32, Account> = BTreeMap::new();
    for row in &accounts {
        let user_id = row.try_get::<_, i32>(0).map_err(read_failure)? as u32;
        by_user.insert(
            user_id,
            Account {
                user_id,
                collateral: row.try_get(1).map_err(read_failure)?,
                positions: Vec::new(),
                frozen: Vec::new(),
            },
        );
    }
    for row in &positions {
        let position = position_from_row(row).map_err(read_failure)?;
        if let Some(account) = by_user.get_mut(&position.user_id) {
            account.positions.push(position);
        }
    }
    for row in &frozen {
        let user_id = row.try_get::<_, i32>(0).map_err(read_failure)? as u32;
        let order_id_hi = row.try_get::<_, i64>(1).map_err(read_failure)? as u64;
        let order_id_lo = row.try_get::<_, i64>(2).map_err(read_failure)? as u64;
        if let Some(account) = by_user.get_mut(&user_id) {
            account.frozen.push(Frozen {
                order_id: format!("{order_id_hi:016x}{order_id_lo:016x}"),
                symbol_id: row.try_get::<_, i32>(3).map_err(read_failure)? as u32,
                amount: row.try_get(4).map_err(read_failure)?,
            });
        }
    }
    Ok(by_user.into_values().collect())
}

async fn do_seed(url: &str, count: u32, collateral: i64) -> Result<(), Failure> {
    let mut client = database::open(url).await?;
    let failed = |error: tokio_postgres::Error| {
        Failure::runtime(format!("failed to seed the accounts: {error}"))
    };
    let last = i32::try_from(count)
        .map_err(|_| Failure::misuse(format!("--count {count} is above {}", i32::MAX)))?;
    let tx = client.transaction().await.map_err(failed)?;
    tx.execute(
        "INSERT INTO accounts (user_id, collateral, version) \
         SELECT user_id, $2::bigint, 0 FROM generate_series(1, $1::int) user_id \
         ON CONFLICT (user_id) DO UPDATE SET \
         collateral = EXCLUDED.collateral",
        &[&last, &collateral],
    )
    .await
    .map_err(failed)?;
    tx.execute(
        "DELETE FROM frozen_orders WHERE user_id BETWEEN 1 AND $1::int",
        &[&last],
    )
    .await
    .map_err(failed)?;
    tx.commit().await.map_err(failed)
}

/// Upserts accounts for users 1 to `count` with `collateral` (an existing
/// account gets the collateral, its version stays) and clears their frozen
/// orders, so a crashed run leaves no reservation that blocks an order with
/// insufficient margin. Prints `{"accounts","collateral"}` under `--json`.
pub fn seed(url: &str, count: u32, collateral: i64, is_json: bool) -> Outcome {
    block_on(do_seed(url, count, collateral))?;
    if is_json {
        println!("{}", json!({ "accounts": count, "collateral": collateral }));
    } else {
        println!("seeded {count} accounts with collateral {collateral}");
    }
    Ok(())
}

/// Prints one user's account; a user without an `accounts` row is a
/// `Runtime` failure.
pub fn show(url: &str, user_id: u32, marks: &Marks, is_json: bool) -> Outcome {
    let accounts = block_on(async {
        let client = database::open(url).await?;
        load_accounts(&client, Some(user_id)).await
    })?;
    let Some(account) = accounts.first() else {
        return Err(Failure::runtime(format!(
            "user {user_id} has no account (seed one with `account seed`)"
        )));
    };
    if is_json {
        println!("{}", account.to_json(marks)?);
    } else {
        print!("{}", account.to_text(marks)?);
    }
    Ok(())
}

/// What `risk overview` prints beyond the accounts.
#[derive(Debug)]
pub struct Overview {
    pub accounts: Vec<Account>,
    pub insurance: Vec<(u32, i64, i64)>,
    pub liquidations: Vec<Value>,
}

impl Overview {
    pub fn insurance_total(&self) -> Result<i64, Failure> {
        self.insurance
            .iter()
            .try_fold(0i64, |total, (_, balance, _)| total.checked_add(*balance))
            .ok_or_else(|| Failure::runtime("insurance fund total overflows"))
    }

    /// `{"users":[Account::to_json],"insurance":{"funds":[{"symbol_id",
    /// "balance","version"}],"total"},"liquidations":[row]}`; a liquidation
    /// row has the columns of `liquidations`, newest first.
    pub fn to_json(&self, marks: &Marks) -> Result<Value, Failure> {
        let mut users = Vec::with_capacity(self.accounts.len());
        for account in &self.accounts {
            users.push(account.to_json(marks)?);
        }
        let funds: Vec<Value> = self
            .insurance
            .iter()
            .map(|(symbol_id, balance, version)| {
                json!({ "symbol_id": symbol_id, "balance": balance, "version": version })
            })
            .collect();
        Ok(json!({
            "users": users,
            "insurance": { "funds": funds, "total": self.insurance_total()? },
            "liquidations": self.liquidations,
        }))
    }

    pub fn to_text(&self) -> Result<String, Failure> {
        let mut text = String::from("  user  collateral  frozen  open_positions\n");
        for account in &self.accounts {
            text.push_str(&format!(
                "{:>6}  {}  {}  {}\n",
                account.user_id,
                account.collateral,
                account.frozen_total()?,
                account.positions.len()
            ));
        }
        text.push_str(&format!(
            "{} accounts, insurance fund {}, {} liquidations shown\n",
            self.accounts.len(),
            self.insurance_total()?,
            self.liquidations.len()
        ));
        Ok(text)
    }
}

async fn load_overview(url: &str) -> Result<Overview, Failure> {
    let client = database::open(url).await?;
    let accounts = load_accounts(&client, None).await?;
    let funds = client
        .query(
            "SELECT symbol_id, balance, version FROM insurance_fund ORDER BY symbol_id",
            &[],
        )
        .await
        .map_err(read_failure)?;
    let mut insurance = Vec::with_capacity(funds.len());
    for row in &funds {
        insurance.push((
            row.try_get::<_, i32>(0).map_err(read_failure)? as u32,
            row.try_get::<_, i64>(1).map_err(read_failure)?,
            row.try_get::<_, i64>(2).map_err(read_failure)?,
        ));
    }
    let rows = client
        .query(
            "SELECT row_to_json(l)::text FROM liquidations l \
             ORDER BY timestamp_ns DESC LIMIT $1",
            &[&LIQUIDATIONS_SHOWN],
        )
        .await
        .map_err(read_failure)?;
    let mut liquidations = Vec::with_capacity(rows.len());
    for row in &rows {
        let text: String = row.try_get(0).map_err(read_failure)?;
        liquidations.push(serde_json::from_str(&text).map_err(read_failure)?);
    }
    Ok(Overview {
        accounts,
        insurance,
        liquidations,
    })
}

/// Prints every account with its open positions and frozen orders, the
/// insurance fund and the last liquidations.
pub fn overview(url: &str, marks: &Marks, is_json: bool) -> Outcome {
    let overview = block_on(load_overview(url))?;
    if is_json {
        println!("{}", overview.to_json(marks)?);
    } else {
        print!("{}", overview.to_text()?);
    }
    Ok(())
}
