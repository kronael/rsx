# SPEC — rsx-core (shared computational core)

The model shared by RSX Go frontends, as a standalone importable Go module:
the order-book folds, the heatmap, the market-data decoder and the order
codec. `rsx-glass` is its consumer; the frontend's render target, keymap,
connectors and chrome live in the frontend (`rsx-glass/specs/`).

## 1. Decisions

| Question | Decision |
|---|---|
| Packaging | Standalone Go module `rsx-core/` with its own `go.mod` (`go 1.24.0`, no `require`, no `go.sum`). Consumers `require rsx-core v0.0.0` and `replace rsx-core => ../rsx-core`. |
| Contents | Exactly three packages: `book`, `wire`, `feed`. |
| What stays in a frontend | Render target (`Scene`, `Grid`, pixels), keymap, connectors (`live/`), test doubles (`mockx/`), chrome. |
| Wire encoder | Core is decoder-only for market data, matching the real producer (`rsx-marketdata/src/wire.rs`), which is the byte authority. The Go mock encoder lives in `rsx-glass/mockx/`. |

```
        rsx-core   (book, wire, feed — pure model, zero external deps)
           |
        rsx-glass  (Scene/Grid render target, live/, mockx/)
```

## 2. Core purity

`wire` depends on nothing; `book` depends on `wire`; `feed` depends on `book`
and `wire`. A frontend may import core; core never imports a frontend.

- Clock-free: no `time.Now` / `time.Since` / `time.After` in `book`/`wire`/`feed`.
  `book.Heatmap.Ingest(…, fromNs, toNs int64)` and `Persistence.AgeNs(px,
  nowNs)` take the clock as a parameter — it is injected, never read. (`time`
  is imported only for the `time.Duration` type in `book/heatmap.go` and a
  timestamp field in `wire/order.go`.)
- Device-free: no `os` / `net` / `rand` / `websocket` in any of the three.
- Render-free: no UI library, no `image/color`, no ANSI. Zero third-party
  imports of any kind — pure stdlib.

## 3. The model → render seam

`Scene` and `Grid` are NOT core — each frontend owns its render target, built
from the core model.

- `rsx-glass/grid.go` defines `Grid`/`Cell` with `Bg/Fg color.RGBA`; colours are
  resolved to pixels in the Grid builder, a render concern.
- `rsx-glass/heat.go` defines `Scene`, a projection holding `book.Row` etc., and
  `Compose(g *Grid, s Scene)` paints the Grid.
- `rsx-glass/feed.go`'s `Feed.Scene()` snapshots the core model into that `Scene`.

The seam is: core model → frontend Scene → frontend render target. What is
shared is the model triad only.

## 4. Public API surface

Keep it minimal and stable: a changed signature breaks a byte-locked golden.

**`rsx-core/wire`** — market-data types and codec, order encode, private-event fold:
- Types: `Side` (`Buy`/`Sell`), `Tif` (`Gtc`/`Ioc`/`Fok`), `Level`, `Bbo`,
  `Snapshot`, `Delta`, `MdTrade`, `MdHeartbeat`, `OrderReq`.
- MD decode: `DecodeMd(frame []byte) (any, error)`.
- Order side: `Cid`, `EncodeNew`, `EncodeCancel`, `IsHeartbeat`, `OidTo64`,
  `RttUnknown`; private-event fold `Folder`/`NewFolder` + `Accepted`/`Done`/
  `Rejected`/`Fill`.

**`rsx-core/book`** — the pure model (clock injected as `nowNs`/`fromNs`/`toNs`):
- `Book` (order-book folds), `Heatmap` + `Row`/`LevelSample`/`TradeSample`/
  `AgeSource` (binning, fisheye, log-time compression, `FarSpan`),
  `Persistence`/`NewPersistence` (age of resting liquidity), `Position`
  (fills → PnL), `Tape`/`TapeEntry` (prints), `Window`/`Sample`/`NsUnknown`/
  `MaxSamples` (latency percentiles), `SeqTracker` (gap detection),
  `FoldBasis`/`BasisDecayShift` (stable ramp basis), session-open labels.

**`rsx-core/feed`** — transport-agnostic link, latency and venue messages and
the order sink interface: `GwUp/GwDown/MdUp/MdDown`, `Latency`, `Submitter`
(`Submit`/`Cancel`), `VenueMsg`/`VenueUp`/`VenueDown`.

## 5. Where connectors and test doubles live

- `rsx-glass/live/` is a WebSocket market-data connector (`coder/websocket`,
  network I/O) and stays in the frontend: it would break the device-free rule.
- `rsx-glass/mockx/` is a mock-exchange WS server plus the Go mirror of the
  Rust MD encoder; test infrastructure, network-bound, stays in the frontend.
- Contract tests (`mockx/contract_test.go`, `live/contract_live_test.go`,
  `live/integration_test.go`) assert the glass mock's frames decode through
  core's real `wire.DecodeMd` and fold through core's `book`. They keep
  importing `rsx-core/wire` and `feed`.

## 6. Verification

```
cd rsx-core  && go build ./... && go vet ./... && go test ./...   # model + wire goldens
cd rsx-glass && make all                                           # fmt/vet/build/headless
```

Goldens that stay green (byte-locked):

- rsx-core: `TestDecodeMdBboGoldenBytes`, `TestDecodeMdSnapshotGoldenBytes`,
  `TestDecodeMdDeltaGoldenBytes`, `TestDecodeMdTradeGoldenBytesAbsentTakerSide`,
  `TestDecodeMdHeartbeatGoldenBytes` (`rsx-core/wire/md_test.go`).
- rsx-glass: `soft` reference PNGs (`rsx-glass/soft/testdata/*.png`), `mockx`
  byte vectors (`rsx-glass/mockx/testdata/*.bin`), and the alloc==0 render test.

Env-gated: `cd rsx-glass && make contract-live` (needs `RSX_MD_URL`).

## 7. Invariants

- The model is UI-agnostic and clock-free: no clock, no device, no render, no
  third-party dependency.
- Any cleanup of the model is its own change against `rsx-core`, with the
  goldens above green before and after.
- `rsx-cast` (Rust, frozen) is out of scope.
