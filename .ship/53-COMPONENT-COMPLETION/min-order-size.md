# Minimum order size: plan

Founder decision, 2026-10-21 (`PLAN.md` § Founder decisions): no cap on
resting orders. Every order instead has a minimum size, about 1 USD of
notional or a per-token minimum in the symbol config, so a flood of resting
orders costs real money. This file is the build plan. Nothing here is built
yet. Line references are to the tree at `d213f031`.

## What the code has today

- The column exists. `symbol_config_schedule.min_notional BIGINT` (nullable)
  and `max_order_qty BIGINT` are in `rsx-risk/migrations/005_symbol_config.sql:28-29`.
  `symbol_config_applied` has the same columns (`:55-56`). The seed
  (`006_seed_symbols.sql`) sets `min_notional = 1000` for all eight presets.
  Nothing in Rust reads either column: risk's `SYMBOL_CONFIG_COLUMNS`
  (`rsx-risk/src/config.rs:420`) selects margin, leverage and fees only. The
  ME's `poll_scheduled_configs` and `write_applied_config`
  (`rsx-matching/src/config.rs`) select and write tick, lot and decimals only,
  so `symbol_config_applied.min_notional` is always NULL. The playground's
  schedule form copies `c.min_notional` forward unchanged
  (`rsx-playground/server.py`, `/api/config/schedule`).
- Units. Prices are raw `10^-price_decimals` quote units and quantities are
  raw `10^-qty_decimals` base units (`rsx_types::validate_order` checks
  `price % tick_size`, so the wire carries raw units that are tick multiples,
  not tick counts). Risk's `order_notional = price × qty`
  (`rsx-risk/src/margin.rs:122`) is in `10^-(price_decimals+qty_decimals)`
  quote units. Collateral is compared against that same product. All eight
  presets have `price_decimals + qty_decimals = 10` (BTC 2+8, ETH 2+8, SOL 4+6,
  memes 6+4), so the seeded `1000` is 1e-7 USD and 1 USD is `10^10`.
- Risk is the only process that computes order notional. It does so in i128
  and saturates to `i64::MAX` on overflow (spec 47 § 2.1). The rejects map to
  the wire in `rsx-risk/src/cast_loop.rs:497-500`.
- No market orders. Every order carries a positive limit price (`OrderRequest`,
  `rsx-messages/src/lib.rs:405`; tif GTC/IOC/FOK, `rsx-types/src/lib.rs:41`).
- Liquidation orders are IOC + reduce-only + `is_liquidation`
  (`rsx-risk/src/liquidation.rs:307-308`) and go through `process_order`
  (`rsx-risk/src/shard.rs:223`). `check_order` returns `Ok(0)` for
  `is_liquidation` and then for `reduce_only` *before* it looks up the symbol
  (`margin.rs:111-121`).
- WAL replay rebuilds freezes with `replay_freeze_order` and does not
  re-run the pre-trade check (`margin.rs:377-397`). A changed minimum cannot
  change recovery.
- The gateway takes tick and lot from env (`rsx-gateway/src/config.rs:46-58`,
  GW-SYMBOL-CONFIG-FROM-ENV) and passes risk's reason byte through untouched
  (`route.rs`, `count_reject_code`). Its reject metrics are indexed by
  `FailureReason` code, with `OTHER_SLOT = FillOrKill + 1`
  (`rsx-gateway/src/metrics.rs:19`).

## Design

### Where the minimum lives: `min_notional`, already in the schema

- **No new column.** `min_notional` is already in `symbol_config_schedule`.
  The new migration is a data migration: it adds a new config version row
  per symbol (the active row is never updated, `rsx-risk/CLAUDE.md`).
- **Unit:** the same raw unit as `price × qty`,
  `10^-(price_decimals + qty_decimals)` quote (USD). "About 1 USD" is
  `10^(price_decimals + qty_decimals)`, which is `10_000_000_000` for every
  preset. Storing it in risk's own notional unit makes the hot-path check a
  single i128 compare, with no scaling and no decimals lookup. The playground
  converts to and from USD for display.
- **"Per-token minimum" = that symbol's own `min_notional` row.** The column
  is already per symbol, so a token whose minimum should differ from 1 USD
  gets a different value in a new version row.
- **No `min_qty`.** A quantity floor does not follow price. A 1 USD floor in
  PEPE units is worth 0.01 USD after a 100x drop, so it stops pricing the
  flood exactly when the token gets cheap. `lot_size` is already the
  structural quantity floor (`qty > 0 && qty % lot == 0`). A second
  structural floor beside tick and lot would make the ME and the gateway
  read a new column: the ME's applied-config path does not carry it and the
  gateway's config is env. The size floor would then have two owners. If the
  founder wants a token-unit floor anyway (see Flags), it goes in risk next to
  the notional check, not in the ME or the gateway.
- **Validation at load:** risk requires `min_notional` NOT NULL and `> 0` on
  every row it applies. This uses the same "column is NULL → InvalidData" path
  as the margin columns (`config.rs:470`), read as `i64` (BIGINT, not the
  INT reader). A bad row fails the boot, or on live reload leaves the symbol
  on its previous version. This is how "every order has a minimum" is
  enforced. No DDL constraint: the loader is the one owner.
- `max_order_qty` stays unread. It is out of scope and named only so nobody
  wires it in by accident here.

### Who enforces it: risk, at its notional boundary

Risk checks `order_notional < min_notional` against the order's **limit
price**, inside `PortfolioMargin::check_order`. Why risk and not the gateway:

1. Notional has one owner. Risk already computes `price × qty` in i128 with
   overflow handling (spec 47 § 2.1). A gateway notional check would be a
   second implementation of the same product.
2. Config is right in risk now. Risk reads `symbol_config_schedule` through
   one apply path for boot, promotion and live reload (`config.rs`,
   `apply_symbol_config`). The gateway's config is env and never moves
   (GW-SYMBOL-CONFIG-FROM-ENV). A gateway check would need that fixed first,
   and would still race CONFIG_APPLIED (47 § 4.1).
3. The crate rules already say so: `rsx-risk/CLAUDE.md` ("enforced here from
   config") and `rsx-gateway/CLAUDE.md` ("enforced by risk").
4. The limit price is the right price. Frozen margin is
   `limit-price notional × im_rate + fee` (`calculate_margin_needed`), so the
   floor and the money it prices are computed on the same number. A
   mark-price check would let a far-from-market buy rest with less frozen
   margin than the floor implies.

What the floor buys. Each resting order freezes at least
`min_notional × im_rate`, so one account can rest at most
`collateral / (min_notional × im_rate)` orders. At 1 USD that is 0.05 USD per
order for large caps (5% IM) and 0.10 USD for small caps (10%). One million
resting orders lock 50k to 100k USD of collateral. The money is locked, not
spent: fees are charged only on fills.

Check order in `check_order` (spec 28 § 6 is renumbered to match):

1. `is_liquidation` → `Ok(0)`. This stays first. The liquidator must be able
   to close dust left by partial fills.
2. Symbol lookup → `SymbolNotConfigured`. This **moves ahead of** the
   reduce-only exemption, because the size check needs the params. The
   behaviour change is nil in practice: no position can exist on a symbol
   without an active row, because rows are append-only.
3. **Size floor** → `OrderTooSmall` when `price as i128 * qty as i128 <
   min_notional as i128`. Exemption: `reduce_only && tif != GTC`. An IOC or
   FOK order never rests, so a sub-minimum reduce-only IOC closes a dust
   position without opening a flood. A GTC reduce-only order must meet the
   floor. Exempting it would allow N resting zero-margin orders against one
   dust position.
4. `reduce_only` → `Ok(0)` (unchanged).
5. Leverage cap, then margin (unchanged). The notional computed in step 3 is
   reused, so no second multiply is added.

Hot path: one compare on a product already being computed. No allocation, no
new branch on the accept path beyond that compare.

### Readers: the gateway and matching do not enforce it

- **Matching:** no change. The ME enforces structural invariants only
  (`rsx-matching/CLAUDE.md` trust boundary). It never reads `min_notional`.
- **Gateway:** no check. GW-SYMBOL-CONFIG-FROM-ENV does **not** have to be
  fixed first, because enforcement does not depend on the gateway's config.
  It is fixed in the gateway component (queue #3), and that fix should add
  `min_notional` to `/v1/symbols` so clients can size orders. Read it from
  `symbol_config_schedule` by the applied version (risk's shape), not from
  `symbol_config_applied`, where the ME writes NULL.
- **Playground:** a reader of risk's Postgres, like the Config tab today. Its
  `/v1/symbols` (currently static `runtime.py` SYMBOLS) adds `min_notional`
  from the active row. The maker reads it there through `RSX_SYMBOLS_URL`.

### Reject code: add `ORDER_TOO_SMALL = 14`

Spec 18 § FailureReason has no fitting code. `INVALID_LOT_SIZE` means
misaligned and `INSUFFICIENT_MARGIN` means not enough money. Reusing either
breaks one-code-per-condition, and the client could not tell the user what to
change. Add `FailureReason::OrderTooSmall = 14` and
`RejectReason::OrderTooSmall` in risk, mapped in `cast_loop.rs`. The client
sees a `U` with status 3 and reason 14. This is additive: unknown codes
already land in the gateway's `rejects_other` slot until the slot table
grows. It is a wire addition, and `FILL_OR_KILL = 13` went to the founder,
so this goes too (see Flags).

### Resting orders below a raised minimum: grandfathered

These orders stay in the book until they fill or are cancelled, and the floor
applies to new orders only. This matches the tick/lot precedent (47 § 3.1,
§ 3.2: "Existing resting orders STAY"). Risk holds no book, the ME does not
read the floor, and the cost of those orders was already paid in frozen
margin. A partial-fill remainder below the floor also rests: its full
original size passed the floor. A live raise reaches risk on the next
CONFIG_APPLIED, which is up to the ME's 10-minute poll (spec 19). Sweeping
old orders would need an ME cancel path with `CANCEL_REASON_SYSTEM`. That is
the alternative listed in Flags, not this plan.

### Playground config tab and seed

- **Config tab:** add a "min order (USD)" column, shown as
  `min_notional / 10^(price_decimals + qty_decimals)`. Add a form field that
  takes USD, converts to the raw value as `round(usd × 10^(pd+qd))` (an
  integer, `> 0`, otherwise 400), and writes it in the scheduled row instead
  of copying `c.min_notional`. The Matching/Config docs (`docs/tabs.md`,
  `docs/api.md`) name the field and its unit.
- **Order result:** reason 14 renders as `ORDER_TOO_SMALL` wherever the
  playground labels FailureReasons (`server.py`/`pages.py` already label
  `FILL_OR_KILL`).
- **Seed, `007_min_order_notional.sql`** (migration id
  `007_min_order_notional`; the existing ids do not follow file names, e.g.
  `005` is `001_symbol_config`, so check that the id is unique). For each
  symbol, take its **highest-version** row. If its `min_notional` IS NULL or
  is below `10^(price_decimals + qty_decimals)`, insert version `max + 1`
  copying every column, with `min_notional = (10::numeric ^ (pd+qd))::bigint`,
  the copied `effective_at_ms`, and `created_at_ms = now`. Copying the
  highest version, not the active one, keeps a future-dated version from
  being overtaken. The rule only raises the floor, so an operator-set higher
  per-token minimum is kept. It is version-guarded, one `DO` block, and
  idempotent (`migrations/CLAUDE.md`). Result for the presets: v2 rows with
  `min_notional = 10_000_000_000` (1 USD) for ids 1, 2, 3, 10-14.

## Blast radius the steps must cover

- Order generators would silently fall below 1 USD. The maker quotes
  `max(lot, RSX_MAKER_QTY × lot)` with a default of 10 lots
  (`market_maker.py:536-538`). For BTC that is 1e-5 BTC, about 0.5 USD.
  `stress_client.py:162` sends one lot. `rsx-cli/src/bench_probe.rs`,
  `demo_scenario.py`, `me_demo.py`, `scripts/demo-trade.sh`, `scripts/smoke.sh`
  and the playground tests (11 `qty=1` hits) all place orders. Each must size
  to the floor at its own limit price:
  `qty = max(qty, ceil(min_notional / price / lot) × lot)`.
- **Benches would measure the reject path.** If `perf-load` / `perf-e2e-gate`
  orders fall below the floor, risk answers without the ME. The round trip
  then gets shorter and reads as an improvement. Gate: `rejects_order_too_small`
  in gateway `/metrics` is 0 after a `make perf-load` run. The risk benches
  build `SymbolRiskParams` literals (25 sites, incl. `benches/*`) and must set
  `min_notional` so they stay on the accept path.
- A shared box. At plan time another session has uncommitted playground edits
  (`pages.py`, `server.py`, play specs). The playground steps start only after
  that work is committed. Stage by path.

## Steps

One sonnet agent and one commit each. The parent reads each diff before the
next step. Run after rsx-matching (#2) closes, as the first block of the
gateway component (#3): step 3 is GW-RESTING-CAPACITY and step 1 touches
gateway metrics.

1. **Wire code.** `FailureReason::OrderTooSmall = 14` (rsx-types and its
   `types_test` pin); spec 18 table and the RISK_REJECTED list; spec 49 and
   `rsx-glass/specs/12-contract-gateway.md` reason tables; gateway
   `REJECT_NAMES` gets `rejects_order_too_small` (OTHER_SLOT moves to 15);
   playground reason labels.
   `feat(types): Add the ORDER_TOO_SMALL failure reason`.
   Verify: `cargo test -p rsx-types -p rsx-gateway` (incl.
   `handler_metrics_test`, `metrics_test`), `make lint`, `make fmt-check`;
   `grep -rn FILL_OR_KILL` shows every table that has 13 also has 14.
2. **Risk enforcement.** Read `min_notional` (i64, NOT NULL, `> 0`) into
   `SymbolRiskParams`. Add the check order above in `check_order`, with
   `RejectReason::OrderTooSmall` mapped to wire 14. Set `min_notional`
   explicitly in every fixture and bench (accept-path benches use a value the
   bench orders clear). Update spec 28 § 6, spec 47 § 2 (new § 2.5), the § 5
   layer table and sequence step 4, and the pre-trade section of risk
   `ARCHITECTURE.md`. Tests: below → `OrderTooSmall` with no freeze left;
   exactly at the floor → accepted; reduce-only IOC below → accepted;
   reduce-only GTC below → rejected; `is_liquidation` below → accepted;
   overflowing notional → `InsufficientMargin`, not `OrderTooSmall`; a live
   reload to a higher floor applies to the next order; over sockets
   (`shard_e2e_test`), ORDER_FAILED reason 14 reaches the gateway side; PG
   (`symbol_config_test`), `min_notional` is loaded and a NULL row fails the
   load naming the column.
   `feat(risk): Reject orders below the symbol's minimum notional`.
   Verify: `cargo test -p rsx-risk`, `make integration`, `make lint`,
   `make fmt-check`, `make test`, and `make perf-gate` on the risk benches
   (within 10%). The seeded `1000` (1e-7 USD) changes no live behaviour, so
   the playground stays green.
3. **Gateway: resting orders off the pending cap.** Today `PendingOrders`
   counts `Resting` entries against `max_pending`
   (`rsx-gateway/src/pending.rs:45`). One user with 10k resting 1 USD orders
   (about 500 USD locked) would OVERLOAD every other user. That is a resting
   cap in all but name, which contradicts the decision. Count only
   `Submitted` toward capacity: `mark_resting` and `remove` adjust the
   in-flight count by stage. Update README/ARCHITECTURE pending, the spec 29
   § Ingress Backpressure wording, and remove GW-RESTING-CAPACITY from
   BUGS.md. Tests: capacity-many resting entries, then another user's order
   is accepted; a full in-flight set still answers `E[5]`.
   `fix(gateway): Count only in-flight orders against the pending cap`.
   Verify: `cargo test -p rsx-gateway`, `make lint`, `make fmt-check`.
4. **Playground reader.** The active-config SQL selects `min_notional`. The
   Config tab gets the USD column and form field (USD → raw, `> 0`).
   `/api/config/schedule` writes it. `/v1/symbols` adds `min_notional` from
   the active row, omitted when Postgres is down. `docs/tabs.md` and
   `docs/api.md`. Tests: API test schedules 2 USD on BTC and reads raw
   `2×10^10` back; the `/v1/symbols` contract; the play spec for the Config
   column.
   `feat(playground): Show and schedule the minimum order notional`.
   Verify: `make gate`.
5. **Order generators to the floor.** Maker, stress client, `bench_probe`,
   demo scripts, smoke, and playground and Rust integration test order
   sizes, using one sizing helper per language with a unit test (ceil to
   lot, per limit price, bid and ask separately).
   `fix(playground): Size generated orders to the symbol minimum`.
   Verify: helper unit tests; schedule a 1 USD floor on every preset through
   the Config tab on a running cluster, then `make gate` green and
   `rejects_order_too_small` 0 on gateway `/metrics` with the maker running.
6. **Seed.** `rsx-risk/migrations/007_min_order_notional.sql` plus its
   `schema.rs` line. Update `migrations/CLAUDE.md` (catalog paragraph names
   `min_notional` and its unit), spec 19 (unit, owner = risk, readers), and
   the `006` comment if it describes the value. Integration tests: clean
   slate → every preset's active row has `10^(pd+qd)`; migrations run twice
   is a no-op; a pre-existing operator v2 → 007 writes v3; a row already
   above 1 USD is left alone.
   `feat(risk): Seed a 1 USD minimum order notional for the presets`.
   Verify: `make integration`, `make gate`, `make perf-load` with
   `rejects_order_too_small = 0`, `make perf-e2e-gate`.
7. **Close-out.** PLAN.md ledger and status, `FEATURES.md` row, BUGS.md
   entries for the flags below that the founder has not decided, diary.
   `docs(ship): Close the minimum order size work`.
   Verify: `git log --oneline` shows steps 1-6; BUGS.md has no
   GW-RESTING-CAPACITY.

## Flags: founder decisions

1. **The ME panics at 65,536 resting orders per symbol.** A book built
   without a checkpoint has `BOOK_ORDER_CAPACITY = 65_536`
   (`rsx-matching/src/recovery.rs:20`). `Slab::alloc` asserts
   `"slab exhausted"` (`rsx-book/src/slab.rs:39-42`), and the GTC
   `insert_resting` path has no guard. At 1 USD and 5% IM, crashing a symbol
   costs about 3.3k USD of locked collateral (6.6k at 10%). That is less than
   the floor is meant to price. This plan does not fix it. Options: (a) the
   ME rejects a resting insert when the slab is full (structural, ME-owned,
   `OVERLOADED`), and the slab is sized for production (spec 21 § 7 allows
   tens of millions); (b) a larger slab only. Recommended: (a). Needs a
   BUGS.md entry (ME-SLAB-EXHAUSTION-PANIC). It is the real "resting cap"
   question.
2. **"Per-token minimum".** Read here as a per-symbol `min_notional` in USD.
   If a token-unit floor ("at least N PEPE") was meant, add a nullable
   `min_qty`, enforced in risk beside the notional check. Default: no
   `min_qty`.
3. **Reduce-only exemption.** Sub-minimum orders are allowed only for
   reduce-only IOC/FOK and liquidation orders. GTC reduce-only must meet the
   floor. Default as stated.
4. **The value.** 1 USD for all eight presets. Large-cap venues often use
   5-10 USD. Default 1 USD.
5. **Grandfathering.** Orders below a raised floor stay (default) or are
   swept by the ME with `CANCEL_REASON_SYSTEM`.
6. **Wire code 14** `ORDER_TOO_SMALL`: a wire addition, the same kind as
   `FILL_OR_KILL = 13`.
7. **The implicit decimals invariant.** Collateral and cross-symbol margin
   sums assume one notional unit, and every preset has
   `price_decimals + qty_decimals = 10` (1e-10 USD). Nothing checks this. A
   symbol with another sum would make portfolio margin, and a hand-entered
   `min_notional`, wrong. The seed uses each row's own sum, so it stays
   correct either way. Proposal: risk refuses a row whose sum differs from
   the exchange's, as a BUGS.md entry.
8. **Step 3 lands in the gateway component.** It is the only way "no cap on
   resting orders" is true at the edge. Resting-entry memory is then bounded
   only by `collateral / (min_notional × im_rate)` across users. At 1 USD
   that bound is loose: 1M entries is about 64 MB.
