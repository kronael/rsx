use crate::account::Account;
use crate::position::Position;
use rtrb::Consumer;
use tokio_postgres::Client;
use tokio_postgres::Error;
use tracing::warn;

#[derive(Clone, Debug)]
pub enum PersistEvent {
    Position(Position),
    Account(Account),
    Fill(PersistFill),
    Tip { symbol_id: u32, seq: u64 },
    FundingPayment(FundingPaymentRecord),
}

#[derive(Clone, Debug)]
pub struct PersistFill {
    pub symbol_id: u32,
    pub taker_user_id: u32,
    pub maker_user_id: u32,
    pub price: i64,
    pub qty: i64,
    pub taker_fee: i64,
    pub maker_fee: i64,
    pub taker_side: u8,
    pub seq: u64,
    pub timestamp_ns: u64,
}

#[derive(Clone, Debug)]
pub struct FundingPaymentRecord {
    pub user_id: u32,
    pub symbol_id: u32,
    pub amount: i64,
    pub rate: i64,
    pub settlement_ts: u64,
}

pub async fn upsert_positions(
    tx: &tokio_postgres::Transaction<'_>,
    positions: &[Position],
) -> Result<(), Error> {
    for p in positions {
        tx.execute(
            "INSERT INTO positions \
             (user_id, symbol_id, long_qty, short_qty, \
              long_entry_cost, short_entry_cost, \
              realized_pnl, last_fill_seq, version) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
             ON CONFLICT (user_id, symbol_id) \
             DO UPDATE SET \
               long_qty = $3, short_qty = $4, \
               long_entry_cost = $5, \
               short_entry_cost = $6, \
               realized_pnl = $7, \
               last_fill_seq = $8, version = $9",
            &[
                &(p.user_id as i32),
                &(p.symbol_id as i32),
                &p.long_qty,
                &p.short_qty,
                &p.long_entry_cost,
                &p.short_entry_cost,
                &p.realized_pnl,
                &(p.last_fill_seq as i64),
                &(p.version as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn upsert_accounts(
    tx: &tokio_postgres::Transaction<'_>,
    accounts: &[Account],
) -> Result<(), Error> {
    for a in accounts {
        tx.execute(
            "INSERT INTO accounts \
             (user_id, collateral, frozen_margin, version) \
             VALUES ($1,$2,$3,$4) \
             ON CONFLICT (user_id) \
             DO UPDATE SET \
               collateral = $2, frozen_margin = $3, \
               version = $4",
            &[
                &(a.user_id as i32),
                &a.collateral,
                &a.frozen_margin,
                &(a.version as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn insert_fills(
    tx: &tokio_postgres::Transaction<'_>,
    fills: &[PersistFill],
) -> Result<(), Error> {
    for f in fills {
        tx.execute(
            "INSERT INTO fills \
             (symbol_id, taker_user_id, maker_user_id, \
              price, qty, taker_fee, maker_fee, \
              taker_side, seq, timestamp_ns) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
            &[
                &(f.symbol_id as i32),
                &(f.taker_user_id as i32),
                &(f.maker_user_id as i32),
                &f.price,
                &f.qty,
                &f.taker_fee,
                &f.maker_fee,
                &(f.taker_side as i16),
                &(f.seq as i64),
                &(f.timestamp_ns as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn upsert_tips(
    tx: &tokio_postgres::Transaction<'_>,
    instance_id: u32,
    tips: &[(u32, u64)],
) -> Result<(), Error> {
    for &(symbol_id, seq) in tips {
        tx.execute(
            "INSERT INTO tips \
             (instance_id, symbol_id, seq) \
             VALUES ($1,$2,$3) \
             ON CONFLICT (instance_id, symbol_id) \
             DO UPDATE SET seq = $3",
            &[
                &(instance_id as i32),
                &(symbol_id as i32),
                &(seq as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn insert_funding(
    tx: &tokio_postgres::Transaction<'_>,
    payments: &[FundingPaymentRecord],
) -> Result<(), Error> {
    for p in payments {
        tx.execute(
            "INSERT INTO funding_payments \
             (user_id, symbol_id, amount, rate, \
              settlement_ts) \
             VALUES ($1,$2,$3,$4,$5)",
            &[
                &(p.user_id as i32),
                &(p.symbol_id as i32),
                &p.amount,
                &p.rate,
                &(p.settlement_ts as i64),
            ],
        )
        .await?;
    }
    Ok(())
}

pub async fn flush_batch(
    client: &mut Client,
    shard_id: u32,
    events: &[PersistEvent],
) -> Result<(), Error> {
    if events.is_empty() {
        return Ok(());
    }

    let mut positions = Vec::new();
    let mut accounts = Vec::new();
    let mut fills = Vec::new();
    let mut tips = Vec::new();
    let mut funding = Vec::new();

    for e in events {
        match e {
            PersistEvent::Position(p) => {
                positions.push(p.clone())
            }
            PersistEvent::Account(a) => {
                accounts.push(a.clone())
            }
            PersistEvent::Fill(f) => {
                fills.push(f.clone())
            }
            PersistEvent::Tip { symbol_id, seq } => {
                tips.push((*symbol_id, *seq))
            }
            PersistEvent::FundingPayment(fp) => {
                funding.push(fp.clone())
            }
        }
    }

    let tx = client.transaction().await?;
    upsert_positions(&tx, &positions).await?;
    upsert_accounts(&tx, &accounts).await?;
    insert_fills(&tx, &fills).await?;
    upsert_tips(&tx, shard_id, &tips).await?;
    insert_funding(&tx, &funding).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn run_persist_worker(
    mut consumer: Consumer<PersistEvent>,
    mut client: Client,
    shard_id: u32,
) {
    let mut buf = Vec::with_capacity(1024);
    loop {
        tokio::time::sleep(
            std::time::Duration::from_millis(10),
        )
        .await;

        buf.clear();
        while let Ok(event) = consumer.pop() {
            buf.push(event);
        }

        if buf.is_empty() {
            continue;
        }

        if let Err(e) =
            flush_batch(&mut client, shard_id, &buf).await
        {
            warn!("persist flush error: {e}");
        }
    }
}
