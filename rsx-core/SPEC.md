# SPEC — rsx-core (shared computational core)

Migration spec for extracting the model both RSX frontends share — `rsx-term`
(the ANSI/TUI) and `rsx-glass` (the GPU/soft renderer) — into a standalone
importable Go module, so neither frontend depends on the other.

**Status:** design, ready for a `sonnet` sub to execute.
**Scope:** DESIGN ONLY here; the executor makes the moves. `rsx-cast` (Rust) is
frozen and out of scope. A concurrent editor owns `rsx-glass/gpu/` +
`rsx-glass/cmd/` — the migration must not depend on their internals (it only
rewrites import paths, which does not touch those packages' logic).

---

## 1. Decision summary (read this first)

| Question | Decision |
|---|---|
| **Core name** | `rsx-core` (runner-up considered: `rsx-model`; rejected because `rsx-core` matches the founder's lean and avoids colliding with the Rust `rsx-book` crate) |
| **Packaging** | **Standalone Go module** `rsx-core/` with its own `go.mod`. Both frontends `require rsx-core v0.0.0` + `replace rsx-core => ../rsx-core`. NOT a single merged monorepo module. |
| **What moves to core** | exactly three packages: `book`, `wire`, `feed` — kept under their current names (`rsx-core/wire`, `rsx-core/book`, `rsx-core/feed`) |
| **What stays in each frontend** | rsx-term: `ui`, `conn`, `news`, `assistant`, `main.go`, `feed/`→moves. rsx-glass: `grid.go`, `glass.go`, `render.go`, `heat.go`, `feed.go` (adapter), `demo.go`, `atlas/`, `soft/`, `live/`, `mockx/` |
| **mockx / live / contract tests** | **STAY in rsx-glass** (frontend). They are network/test-double code (`coder/websocket`, WS mock server) — device-bound, so they violate core's device-free rule. Justified in §7. |
| **The move is a pure import-path prefix swap** | Package names are unchanged (`package wire/book/feed`), so no symbol reference changes — only the module prefix `rsx-term/…` → `rsx-core/…`. This is what keeps every golden byte-identical. |
| **Net effect** | rsx-glass drops `replace rsx-term => ../rsx-term` entirely; rsx-glass no longer imports rsx-term at all. |

The end-state dependency graph:

```
        rsx-core   (book, wire, feed — pure model, zero external deps)
        /      \
  rsx-term    rsx-glass      (siblings; neither imports the other)
```

---

## 2. Why this is clean: the verified boundary

The reuse seam was measured, not assumed. `rsx-glass` imports **exactly three**
`rsx-term` packages and nothing else:

```
$ grep -rn 'rsx-term/' rsx-glass --include='*.go'   # (import lines only)
  rsx-term/book   demo.go feed.go heat.go heat_test.go
  rsx-term/wire   demo.go feed.go feed_test.go heat.go heat_test.go
                  live/live.go live/*_test.go mockx/scenario.go mockx/wire.go
                  mockx/contract_test.go
  rsx-term/feed   feed.go feed_test.go live/live.go live/integration_test.go
```

It never imports `rsx-term/ui`, `rsx-term/conn`, `rsx-term/news`, or
`rsx-term/assistant`. So the TUI render, keymap, venues, news, and chat pane
already sit outside the reuse surface — the `replace => ../rsx-term` drags them
in only as transitive module weight, never as real dependency.

Inside `rsx-term`, those three packages form a **closed, well-ordered
subgraph** (verified from the internal import graph):

```
wire   →  (nothing — leaf; stdlib only)
book   →  wire
feed   →  book, wire
```

None of the three import `ui`/`conn`/`news`/`assistant`. So `{wire, book, feed}`
lifts out whole, with no back-edge into the frontend.

**Core-purity checks (all verified green):**

- Clock-free: no `time.Now` / `time.Since` / `time.After` in `book`/`wire`/`feed`.
  `book.Heatmap.Ingest(…, fromNs, toNs int64)` and `Persistence.AgeNs(px,
  nowNs)` take the clock as a parameter — it is injected, never read. (`time` is
  imported only for the `time.Duration` type in `book/heatmap.go` and a
  timestamp field in `wire/order.go`.)
- Device-free: no `os` / `net` / `rand` / `websocket` in any of the three.
- Render-free: no `charmbracelet/*` (bubbletea/lipgloss), no `image/color`, no
  ANSI. Zero third-party imports of any kind — pure stdlib.

That triad is the computational core: order book folds, heatmap
binning/compression + fisheye + persistence, position/PnL, tape, latency
window, seq tracking, the market-data wire decoder + types, and the order
encoder. All clock-free / render-free / device-free by construction.

---

## 3. The model → render seam (correcting the "Scene/Grid is core" framing)

`Scene` and `Grid` are **NOT** core — they are each frontend's private render
target, built *from* the core model. Be exact about this:

- `rsx-glass/grid.go` defines `Grid`/`Cell` with `Bg/Fg color.RGBA` — colours
  are *resolved to pixels* in the Grid builder. That is a GPU/soft-render
  concern, not a model concern.
- `rsx-glass/heat.go` defines `Scene` (`heat.go:55`) — a projection struct
  holding `book.Row` etc. — and `Compose(g *Grid, s Scene)` paints the Grid.
- `rsx-glass/feed.go`'s `Feed.Scene()` snapshots the core model into that
  `Scene`.
- `rsx-term/ui/` is the symmetric partner: it renders the same core model to
  ANSI cells / lipgloss strings. Its own render target (styled cells) never
  leaves `ui`.

So the seam is: **core model → (frontend-specific Scene/projection) →
(frontend-specific render target: Grid+RGBA for glass, ANSI cells for term).**
Both `Scene` and `Grid` stay in `rsx-glass`; the ANSI render stays in
`rsx-term/ui`. What crosses the seam — and is shared — is the model triad only.

---

## 4. Exact move-list

### 4.1 MOVES to `rsx-core/` (git mv, package name unchanged)

| From | To | Package | What it is |
|---|---|---|---|
| `rsx-term/wire/types.go` | `rsx-core/wire/types.go` | `wire` | `Side`, `Tif`, `Level`, `Bbo`, `Snapshot`, `Delta`, `MdTrade`, `MdHeartbeat`, `OrderReq` |
| `rsx-term/wire/md.go` | `rsx-core/wire/md.go` | `wire` | `DecodeMd(frame []byte) (any, error)` — the MD decoder |
| `rsx-term/wire/md_test.go` | `rsx-core/wire/md_test.go` | `wire` | `TestDecodeMd*GoldenBytes` (byte-vector goldens — travel with the code) |
| `rsx-term/wire/order.go` | `rsx-core/wire/order.go` | `wire` | `Cid`, `EncodeNew`, `EncodeCancel`, `IsHeartbeat`, `OidTo64`, `Folder`, `NewFolder`, `Accepted/Done/Rejected/Fill`, `RttUnknown` |
| `rsx-term/wire/order_test.go` | `rsx-core/wire/order_test.go` | `wire` | folder/encode tests |
| `rsx-term/book/book.go` | `rsx-core/book/book.go` | `book` | `Book` + `ApplySnapshot/ApplyBbo/ApplyDelta/Bbo/BestBid/BestAsk/Spread/Mid/Empty` |
| `rsx-term/book/heatmap.go` | `rsx-core/book/heatmap.go` | `book` | `Heatmap`, `Row`, `LevelSample`, `TradeSample`, `AgeSource`, `Ingest/Rows/Configure/…`, `FarSpan`, fisheye + log-time + size compression |
| `rsx-term/book/persist.go` | `rsx-core/book/persist.go` | `book` | `Persistence`, `NewPersistence`, `ObserveSnapshot/ObserveDelta/AgeNs` |
| `rsx-term/book/position.go` | `rsx-core/book/position.go` | `book` | `Position`, `ApplyFill/Entry/Upnl/Flat` |
| `rsx-term/book/tape.go` | `rsx-core/book/tape.go` | `book` | `Tape`, `TapeEntry`, `Push/Entries/Last` |
| `rsx-term/book/latency.go` | `rsx-core/book/latency.go` | `book` | `Sample`, `Window`, `NsUnknown`, `MaxSamples`, `Add/P50/P99/Min/Len/Recent` |
| `rsx-term/book/seq.go` | `rsx-core/book/seq.go` | `book` | `SeqTracker`, `Observe/Reset/ResetTo` |
| `rsx-term/book/*_test.go` | `rsx-core/book/*_test.go` | `book` | `book_test`, `heatmap_test`, `persist`(none)/`position_test`, `tape_test`, `latency_test`, `seq_test` — model unit tests travel with the model |
| `rsx-term/feed/feed.go` | `rsx-core/feed/feed.go` | `feed` | `GwUp/GwDown/MdUp/MdDown`, `Latency`, `Submitter`, `VenueMsg/VenueUp/VenueDown` |

> Note: `rsx-term/book/persist.go` has no `_test.go`; do not invent one. Move
> only files that exist — enumerate with `git ls-files rsx-term/{wire,book,feed}`
> before moving so the set is exact.

### 4.2 STAYS in `rsx-term/` (frontend: TUI render + venues + news + chat)

`ui/` (all render/keymap/model/view/stream/styles/market/sparkline/news_view +
their tests, incl. the goldens `ui/dom_golden_test.go`,
`ui/stream_golden_test.go`), `conn/` (hyperliquid/live/phoenix/mock/jwt/symbols
— venue connectors, `coder/websocket`), `news/`, `assistant/`, `main.go`,
`hl_smoke_test.go`. These re-point their `rsx-term/{wire,book,feed}` imports to
`rsx-core/…` (§6) but do not move.

### 4.3 STAYS in `rsx-glass/` (frontend: soft/GPU render + feed adapter + test infra)

`grid.go`, `glass.go`, `render.go`, `heat.go` (`Scene` + `Compose`), `feed.go`
(the model-fold adapter), `demo.go`, `atlas/`, `soft/` (+ `soft/testdata/*.png`
goldens), `live/` (WS connector + integration/contract tests),
`mockx/` (mock exchange + `mockx/testdata/*.bin`). These re-point their
`rsx-term/{wire,book,feed}` imports to `rsx-core/…` and drop the
`replace rsx-term` directive (§6).

**Concurrency note:** `rsx-glass/gpu/` and `rsx-glass/cmd/` (owned by another
editor concurrently) are frontend and stay put. If they import
`rsx-term/{wire,book,feed}` at execution time, they get the same prefix rewrite
as every other glass file — nothing else. Re-run the import grep at execution
time to catch any files added since this spec.

---

## 5. Public API surface (keep minimal + stable)

The core exposes exactly what the two frontends already consume. Do **not** add,
rename, or "tidy" any signature during the move — a changed signature breaks a
golden. The surface, per package:

**`rsx-core/wire`** — market-data types + codec, order encode, private-event fold:
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
  `MaxSamples` (latency percentiles), `SeqTracker` (gap detection).

**`rsx-core/feed`** — transport-agnostic link/latency/venue messages + the order
sink interface: `GwUp/GwDown/MdUp/MdDown`, `Latency`, `Submitter` (`Submit`/
`Cancel`), `VenueMsg`/`VenueUp`/`VenueDown`.

API-stability rule for the executor: the diff to any moved `.go` file must be
**import lines only** (`rsx-term/…` → `rsx-core/…`). If a body line changes, the
move is wrong — revert and redo.

---

## 6. Packaging decision (weighed)

**Recommended: standalone module `rsx-core/` with its own `go.mod`.**

`rsx-core/go.mod`:
```
module rsx-core

go 1.24.0
```
No `require` block, no `go.sum` — the triad has zero third-party deps (pure
stdlib). `go 1.24.0` is the lower of the two frontends' directives (term
`1.24.0`, glass `1.25.0`), so both toolchains can import it.

Both frontends gain:
```
require rsx-core v0.0.0
replace rsx-core => ../rsx-core
```
and `rsx-glass` **loses** `require rsx-term v0.0.0` + `replace rsx-term =>
../rsx-term`.

**Why standalone over a single merged monorepo module:**

| | Standalone module (recommended) | Single merged module |
|---|---|---|
| Boundary clarity | Core cannot import a frontend — enforced by the module graph; the DAG is a compile-time fact | Nothing stops a future edit re-coupling model↔render inside one module |
| Consistency with repo | Matches the existing convention — `rsx-term` and `rsx-glass` are already separate modules; glass already uses a `replace` for term | Would force-merge term+glass into one module, breaking that convention and the "crate-per-concern" layout |
| Change size | Additive: new module + prefix rewrites; frontends stay independent modules | Large, invasive: collapse two module roots, reconcile `go.mod`s, re-slug every import in both |
| Cost | `replace` directive friction (two `go.mod`s carry it; `go work` optional) | Coupling risk + a much bigger, riskier diff |

The one real cost of standalone is the `replace`-directive friction — every
consumer must carry `replace rsx-core => ../rsx-core`, and a fresh `go build`
outside the repo can't resolve `v0.0.0`. That friction **already exists** in
this repo (glass ↔ term) and is the accepted pattern, so it is not a new tax.
The founder's lean (standalone-importable over "same project") and the
boundary-clarity win make this the call. An optional top-level `go.work`
(`use ./rsx-core ./rsx-term ./rsx-glass`) can remove the `replace` lines later —
note it, don't require it (out of scope for this migration).

---

## 7. Where mockx / live / contract tests go

**Decision: they STAY in `rsx-glass` (frontend). Do not move them to core.**

- **`rsx-glass/live/`** is a WebSocket market-data connector — it imports
  `github.com/coder/websocket` and does network I/O. Core is device-free by
  contract (§2); putting `live` in core would drag `coder/websocket` into the
  shared module and onto every importer. It is the glass analogue of
  `rsx-term/conn/` (which *also* stays in term). Connectors are frontend.
- **`rsx-glass/mockx/`** is a mock-exchange WS server + a wire **encoder**
  (`mockx/wire.go`) that is the inverse of `wire.DecodeMd`. It is test/dev
  infrastructure, network-bound (`net/http`, `coder/websocket`), so it fails the
  device-free rule too.
- **The wire encoder question (considered, rejected for now):** `mockx/wire.go`
  is the symmetric partner of core's `wire.DecodeMd`, and one *could* promote a
  pure `Encode*` codec into `rsx-core/wire` so both frontends synthesize frames
  from one authoritative codec. Rejected: `rsx-term` ships **no** MD-frame
  encoder (verified — its only encoders are the order-text `EncodeNew`/
  `EncodeCancel`) and doesn't need one; core stays **decoder-only**, matching the
  real producer, which is the byte-authority. Promoting the encoder is a future
  option if a second frontend ever needs to synthesize frames — left as a note,
  not done here (minimal + stable API wins).
- **Contract tests** (`mockx/contract_test.go`, `live/contract_live_test.go`,
  `live/integration_test.go`) assert that the glass mock's frames decode through
  core's real `wire.DecodeMd` and fold through core's `book` model. They are
  glass's tests of glass's mock against the core codec — they belong with the
  code they exercise. They keep importing `rsx-core/wire` (+ `feed`) after the
  rewrite. `mockx/testdata/*.bin` (committed byte vectors) are unchanged by the
  move.

Net: core is the **shared model**, not a shared feed. Each frontend keeps its
own connector + mock (term: `conn/`; glass: `live/` + `mockx/`), both folding
into the one core model.

---

## 8. Migration plan (ordered; goldens green at every checkpoint)

The whole migration is a sequence of **pure import-path prefix rewrites** plus
`go.mod` edits. Because package names never change, the only diff to moved files
is their import lines, so rendered output (ANSI cells, soft PNGs, wire bytes) is
byte-identical — that is the mechanism that keeps the goldens green.

**Move one package at a time, leaf-first (`wire` → `book` → `feed`), and run the
full gate after each.** Incremental (vs big-bang) so a break is isolated to one
small, reversible step. Work in detached HEAD; one commit per step.

### Step 0 — scaffold the module (no code moved yet)
1. `mkdir rsx-core`; write `rsx-core/go.mod` (`module rsx-core` / `go 1.24.0`).
2. Add to **both** `rsx-term/go.mod` and `rsx-glass/go.mod`:
   `require rsx-core v0.0.0` + `replace rsx-core => ../rsx-core`.
   (Harmless while nothing imports it yet; keeps the next steps' `go.mod` edits
   minimal.)
3. Gate: `cd rsx-term && go build ./...` and `cd rsx-glass && go build ./...`
   still pass (empty core module is inert). Commit: `chore(core): scaffold rsx-core module`.

### Step 1 — move `wire` (leaf)
1. `git mv rsx-term/wire rsx-core/wire` (moves `types.go`, `md.go`, `order.go`
   + their `_test.go`).
2. Rewrite import path `rsx-term/wire` → `rsx-core/wire` **everywhere in both
   modules** (incl. inside the moved files — none import `rsx-term/wire`, but
   `book`/`feed` in term still do, and all of `ui`/`conn`/`glass` do):
   ```
   grep -rl '"rsx-term/wire"' rsx-term rsx-glass --include='*.go' \
     | xargs sed -i 's#"rsx-term/wire"#"rsx-core/wire"#'
   ```
3. Gate (both modules): `go build ./... && go test ./...`.
   - Core's own `wire` golden-byte tests (`TestDecodeMd*GoldenBytes`,
     folder/encode tests) now run under `rsx-core` — must be green.
   - rsx-term `ui` goldens (`TestDomViewGolden`, `TestBookViewGolden`,
     `TestDefaultViewUnchanged`) — must stay green.
   - rsx-glass `soft`/`mockx` goldens — must stay green.
   Commit: `refactor(core): move wire package into rsx-core`.

### Step 2 — move `book`
1. `git mv rsx-term/book rsx-core/book` (all `*.go` incl. tests; `book`'s
   internal `import "rsx-term/wire"` lines were already rewritten to
   `rsx-core/wire` in Step 1).
2. Rewrite `rsx-term/book` → `rsx-core/book` in both modules:
   ```
   grep -rl '"rsx-term/book"' rsx-term rsx-glass --include='*.go' \
     | xargs sed -i 's#"rsx-term/book"#"rsx-core/book"#'
   ```
3. Gate (both modules): `go build ./... && go test ./...` — all goldens green.
   Commit: `refactor(core): move book model into rsx-core`.

### Step 3 — move `feed`
1. `git mv rsx-term/feed rsx-core/feed` (its `import "rsx-term/book"` /
   `"rsx-term/wire"` are already core paths from Steps 1–2).
2. Rewrite `rsx-term/feed` → `rsx-core/feed` in both modules:
   ```
   grep -rl '"rsx-term/feed"' rsx-term rsx-glass --include='*.go' \
     | xargs sed -i 's#"rsx-term/feed"#"rsx-core/feed"#'
   ```
3. Gate (both modules): `go build ./... && go test ./...` — all goldens green.
   Commit: `refactor(core): move feed messages into rsx-core`.

### Step 4 — cut rsx-glass's dependency on rsx-term
1. Confirm it's gone: `grep -rn 'rsx-term/' rsx-glass --include='*.go'` returns
   **no import lines** (doc-comment mentions of "rsx-term" are fine).
2. Remove from `rsx-glass/go.mod`: `require rsx-term v0.0.0` and
   `replace rsx-term => ../rsx-term`. `cd rsx-glass && go mod tidy`.
3. Gate: `cd rsx-glass && make all` (fmt-check + vet + build + headless suite,
   incl. `go vet -tags contract ./live/`). Commit: `refactor(glass): drop rsx-term dependency`.

### Step 5 — finalize term module + housekeeping
1. `cd rsx-term && go mod tidy` (drops now-unused indirect deps if any).
2. `cd rsx-core && go build ./... && go test ./...` (core stands alone).
3. `make fmt` in every touched module; re-run each module's test gate once more.
   Commit: `chore(core): tidy module graph after extraction`.

> The `replace rsx-core` lines added in Step 0 to both frontends stay — they are
> how the frontends resolve the local core.

---

## 9. Verification — the exact gate that must be green at every checkpoint

Run all three after every step (fast; the model tests are pure):

```
cd rsx-core  && go build ./... && go test ./...          # model + wire goldens
cd rsx-term  && go test ./...                             # incl. ui goldens
cd rsx-glass && make all                                  # fmt/vet/build/headless
```

Named goldens that MUST stay green (byte-locked — a diff here means the move
mutated something it shouldn't have):

- rsx-term: `TestDomViewGolden`, `TestBookViewGolden`, `TestDefaultViewUnchanged`
  (`rsx-term/ui/dom_golden_test.go`, `rsx-term/ui/stream_golden_test.go`).
- rsx-core (moved from term/wire): `TestDecodeMdBboGoldenBytes`,
  `TestDecodeMdSnapshotGoldenBytes`, `TestDecodeMdDeltaGoldenBytes`,
  `TestDecodeMdTradeGoldenBytesAbsentTakerSide`,
  `TestDecodeMdHeartbeatGoldenBytes` (`rsx-core/wire/md_test.go`).
- rsx-glass: `soft` reference PNGs (`rsx-glass/soft/testdata/*.png`), `mockx`
  byte vectors (`rsx-glass/mockx/testdata/*.bin`), and the alloc==0 render test.

Optional, env-gated (unchanged by this migration; run if creds present):
`cd rsx-glass && make contract-live` (needs `RSX_MD_URL`).

---

## 10. Risks, mitigations, rollback

| Risk | Mitigation |
|---|---|
| A prefix rewrite touches a doc-comment string or a `//go:` directive, not an import | The `sed` targets the quoted import path `"rsx-term/wire"` exactly — comments say `rsx-term/wire` without quotes and are untouched. Verify each step's diff is import-lines-only (`git diff --stat` + spot-check). |
| A moved file's body changes (accidental reformat) → golden flips | Rule §5: moved-file diff is import lines only. `git diff` each moved package; if a body line moved, revert and redo. Run `make fmt` **after** moves, then re-test, so formatting churn is caught by the gate, not shipped silently. |
| Concurrent `rsx-glass/gpu/` + `cmd/` editor adds a new `rsx-term/{…}` import mid-migration | The rewrite is grep-driven (`grep -rl` feeds `sed`), so it catches any file present at execution time. Re-run the import grep in Step 4 before cutting the dep; if `gpu/`/`cmd/` gained a term import, it's rewritten like the rest. Never edit those files' logic — only their import lines. |
| go-version skew (term 1.24, glass 1.25) | Core pins `go 1.24.0` (the lower); both toolchains import it. Don't bump. |
| `go.mod` `replace` forgotten in one frontend → build can't resolve `v0.0.0` | Step 0 adds the `replace` to **both** frontends up front, before any import points at core. The Step-0 gate builds both. |
| Big-bang temptation | Explicitly rejected — move one package per commit so a red gate localizes to one small step. |

**Rollback:** every step is one commit and one reversible prefix rewrite. To
undo step N: `git revert` (or reset) that commit; the inverse `sed`
(`rsx-core/<pkg>` → `rsx-term/<pkg>`) plus `git mv` back restores the prior tree.
Because package names and file bodies never change, rollback is mechanical and
lossless. If the whole migration must be abandoned, revert steps 5→0 in reverse
order; `rsx-glass` regains `replace rsx-term => ../rsx-term` and the tree is
byte-identical to the start.

---

## 11. Invariants respected (out-of-scope guards)

- **rsx-term stays the visual template.** No render/keymap/palette/glyph change
  — `ui/`, `VISUALS.md`, `SCREENS.md` untouched. The model relocates; the look
  does not.
- **The model is UI-agnostic / clock-free** (rsx-term `CLAUDE.md` keeper): this
  migration *depends on* that property and preserves it — core has no clock, no
  device, no render, no third-party dep.
- **Honesty rules, offline-by-default, keyboard-only, byte-locked DOM view** —
  all live in frontend packages (`ui/`, `conn/`), none of which move. Unaffected.
- **`rsx-cast` (Rust, frozen)** — not touched; out of scope.
- **No new features, no API tidy, no signature changes** — pure extraction. Any
  cleanup of the model is a separate, later change against `rsx-core`.
```

---

## How to read this

- **§1** — the decisions, at a glance.
- **§2–§3** — the verified boundary and the model→render seam (why the split is exact).
- **§4–§5** — what moves, what stays, the public API to preserve.
- **§6–§7** — packaging and the mockx/live/contract-test placement, each justified.
- **§8–§9** — the ordered, goldens-green migration steps and the exact gate.
- **§10–§11** — risks/rollback and the invariants this must not regress.
