# The venue seam — an exchange is a plugin, not a fork

A terminal for *one* exchange is easy. The value is one terminal for *many* — and
the trap is that most multi-exchange tools are really N single-exchange tools
stitched together, where every screen knows about "the RSX gateway" and
"Hyperliquid's JSON" and adding a venue means touching everything. The question
this design answers: **how do you add an exchange without touching the render, the
heatmap, or the trading grammar at all?**

## Problem — the N×M fork

If the UI speaks each exchange's native wire format, every feature multiplies by
every venue. The heatmap needs to know HL's `l2Book` shape *and* RSX's protobuf;
the order form needs HL's EIP-712 signing *and* RSX's JSON frames. N exchanges ×
M features, and each new venue is a fork that risks every existing screen.

## Fix — normalise at the edge, speak one language inside

Two small interfaces are the entire seam:

- a **market-data Source** that emits *normalised* events — `wire.Snapshot`,
  `wire.Delta`, `wire.Bbo`, `wire.MdTrade`, all in generic `wire.Level`
  (price/size/count) terms, tagged with a venue name;
- an **order Submitter** — `Submit(OrderReq) error` / `Cancel(cid) error`.

Each exchange is *one file* that implements these and nothing else touches the
outside world:

- `conn/live.go` — the RSX gateway (WS + protobuf market data + JSON orders),
- `conn/hyperliquid.go` — Hyperliquid (its JSON `l2Book`/`trades` mapped into the
  *same* `wire.Snapshot`/`MdTrade` shapes the RSX feed produces),
- `conn/mock.go` — a scripted offline source, no network.

Everything above the seam — the book folds, the log-time heatmap, the render, the
game-entry grammar — consumes the normalised `wire.*` types and never sees an
exchange's raw format. **Adding an exchange is writing a new Source (and,
optionally, a Submitter); the UI changes by zero lines.**

## Read-only and trading are decoupled

A venue registers with a `nil` Submitter when it's market-data-only. Hyperliquid
is read-only today — its market data gives real breadth (~150 perps for the NEWS
overview), but trading there needs EIP-712 order signing with an Ethereum key,
which isn't wired. The UI surfaces that honestly ("read-only venue", trades
blocked), so *watching many venues* and *trading the ones you have keys for* are
independent capabilities, not an all-or-nothing per venue.

## The mock is the proof

`conn/mock.go` is a full alternative Source with no network — it drives the
offline demo. That the demo and a live exchange plug into the *identical* seam is
the proof the abstraction is real and not a leaky pretence: if the offline mock
and Hyperliquid are interchangeable to everything above them, so is the next
exchange.

## The cost it removes

The N×M fork disappears — the terminal is genuinely generic multi-exchange, and
the same normalised model can later drive a GPU/bitmapped frontend unchanged
(the model→render boundary is the *only* thing a new frontend replaces). Trust
boundaries stay clean too: the public market-data Source is untrusted and
validated at the edge (see `honesty.md`), so a lying venue frame can't corrupt
the shared fold.

## The through-line

**Normalise at the edge; speak one language inside.** An exchange becomes a
plugin behind two tiny interfaces, not a fork through the whole codebase — the
same principle that lets the mock, RSX, and Hyperliquid be interchangeable is
what makes the fourth exchange free.

---

*Prior art: the classic ports-and-adapters / hexagonal boundary. Cross-venue
aggregators (Aggr, Velo, Coinalyze) normalise market data similarly — but stop at
*viewing*; keeping a per-venue `Submitter` in the same seam is what lets this
terminal also *trade* the venues it has keys for, on the same axis it watches
them. See `conn/` for the three implementations and `wire/` for the normalised
shapes.*
