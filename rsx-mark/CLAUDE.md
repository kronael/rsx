# CLAUDE.md — rsx-mark

Local to `rsx-mark/`; the root `../CLAUDE.md` applies. Mark aggregates
external exchange feeds into one mark price per symbol and casts it to risk;
every update also goes to its WAL. Specs: `specs/2/15-mark.md`; tests
`39-testing-mark.md`.

## Rules

- A generic relay: read external feeds, keep only the latest value per
  symbol, persist every update in WAL format, cast to the risk shards that
  connect. Not a pub/sub broker; no exchange logic.
- Mark price (external) is for margin and liquidation; index price (the
  book's mid) is for funding, which is the mid-to-mark difference. NEVER
  swap the two.
- Off the critical path on tokio, but pinned on `RSX_MARK_CORE_ID` like
  every other daemon: an unpinned spinner steals a hot core. Ergonomics
  over micro-latency here.
- Mark prices carry their observation age (`RSX_MARK_STALENESS_NS` bounds
  it); a stale mark is reported as stale, never served as fresh.
- With no external feed (dev boxes) the system runs on the index fallback;
  the market maker quotes from the mark/index feed, never from a synthetic
  price.

## Docs

`README.md` (what, `Running`), `ARCHITECTURE.md` (sources, aggregation, cast
+ WAL). Dated numbers live in root `reports/`.
