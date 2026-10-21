---
status: draft
---

# 13 — Contract: market data, venues and news

The market-data contract is how the UI learns the book and the tape: the RSX
public feed (protobuf frames over WebSocket), the read-only venue adapters
(Hyperliquid, Phoenix, the offline mock) that normalise other exchanges into
the same shapes, the gateway's symbol-config read, and the Tree of Alpha
headline stream. The order channel is [12-contract-gateway.md](12-contract-gateway.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.
`rsx-core/wire` (decoder) and `rsx-core/book` (folds) survive; the venue
adapters and the news reader lived in `rsx-term/conn/` and `rsx-term/news/`
and are captured here. Glass already has its own RSX market-data connector
(`rsx-glass/live/`) and a mock exchange (`rsx-glass/mockx/`).

## The venue seam

| ID | Requirement | Source |
|---|---|---|
| MD-01 | Every exchange is one adapter that emits normalised `wire.Snapshot / Delta / Bbo / MdTrade` in generic `wire.Level{Px, Qty, Count}` terms, tagged with a venue name (`feed.VenueMsg`), plus `feed.VenueUp/VenueDown`; and optionally one order Submitter (`Submit(OrderReq) / Cancel(cid)`). Nothing above the seam sees a venue's native format; adding a venue changes zero UI lines. | [doc rsx-term/notes/venue-seam.md], [code rsx-core/feed/feed.go] |
| MD-02 | A venue with no Submitter is read-only: every screen renders, every order is blocked with the reason. Watching many venues and trading the ones with keys are independent capabilities. | [code rsx-term/ui/update.go:394-401, 593-598], [code rsx-term/ui/venue_test.go TestReadOnlyVenueBlocksOrders] |
| MD-03 | RSX is the first-class venue; the others prove the seam. Venue selection: `rsx` (default) · `hyperliquid` / `phoenix` (standalone read-only over that venue) · `both` (RSX + HL) · `all` (RSX + HL + Phoenix) · a comma list. A failed universe fetch for an extra venue warns and continues without it; it never blocks the RSX UI. | [founder 2026-10-21 "RSX is the first-class venue, the others prove the seam"], [code rsx-term/main.go:57-89, 155-218], [founder 2026-07-11 "the terminal is generic MULTI-EXCHANGE (RSX + HL are just two backends)"] |
| MD-04 | The public feed is untrusted (spec 4 §10.4): frames are validated at the adapter; a malformed side or price is skipped, never coerced; a decode error drops one frame, never the connection. | [doc rsx-term/notes/honesty.md rule 6], [code rsx-term/conn/live.go:403-443] |
| MD-05 | Offline by default: the default path makes zero network calls; every dial site (RSX gateway + md, HL, Phoenix, news, assistant, support) sits behind an explicit opt-in and runs in a named goroutine. | [doc rsx-term/CLAUDE.md "Offline by default"] |
| MD-06 | Symbol id 0 is reserved for "unspecified → the primary symbol"; adapters use 1-based ids (HL: universe index + 1; Phoenix: assetId + 1). An id not on any venue renders as an honest `SYM-<id>` stub, never under the primary's name or scale. | [code rsx-term/ui/update.go:669-676], [code rsx-term/conn/phoenix.go:70-85], [code rsx-term/ui/model.go:128-161] |
| MD-07 | Every watched (venue, symbol) folds continuously in the background (book, tape, heatmap, persistence, position), so a symbol or venue hop is instant. | [code rsx-term/ui/update.go:641-701], [code rsx-term/ui/update.go:352-362] |

## RSX public feed

Connection: `ws://…:8180` default; one subscribe frame per watched symbol on
connect and on every reconnect:

```go
// rsx-term/conn/live.go:207-223 (exact)
func (g *LiveGateway) dialMd(ctx context.Context) (*websocket.Conn, error) {
	conn, _, err := websocket.Dial(ctx, g.mdURL, nil)
	if err != nil {
		return nil, err
	}
	for _, id := range append([]uint32{g.symbolID}, g.watch...) {
		frame := fmt.Sprintf(`{"S":[%d,%d]}`, id, mdChannels)
		wctx, cancel := context.WithTimeout(ctx, writeTimeout)
		err := conn.Write(wctx, websocket.MessageText, []byte(frame))
		cancel()
		if err != nil {
			_ = conn.CloseNow()
			return nil, err
		}
	}
	return conn, nil
}
```

`mdChannels = 7` = bbo (1) | depth (2) | trades (4). Inbound control is JSON
text (`{"S":[sym,ch]}` subscribe, `{"X":[sym,ch]}` unsubscribe, `{"H":[ts_ms]}`
heartbeat echo); outbound is binary protobuf
[code rsx-term/conn/live.go:43-45], [doc specs/2/49-webproto.md "Market-data subscribe"].
The server's heartbeat MUST be echoed as `{"H":[<ts_ms>]}`: the marketdata
server treats the echo as this client's liveness
[code rsx-term/conn/live.go:403-443].

Frame: `[u32 big-endian body length][MdFrame protobuf body]`; MdFrame is a
oneof — tag 1 Bbo, 2 Snapshot, 3 Delta, 4 Trade, 5 Heartbeat; proto3 zero
omission (an absent scalar is 0); only wire types 0 and 2 legal. Decoder
(hand-derived, no codegen) [code rsx-core/wire/md.go:182-264]:

```go
// rsx-core/wire/md.go:211-261 (exact; field tags per message)
		case 1: // Bbo
			return Bbo{
				SymbolID: uint32(u(f, 1)),
				BidPx:    asI64(u(f, 2)),
				BidQty:   asI64(u(f, 3)),
				BidCount: uint32(u(f, 4)),
				AskPx:    asI64(u(f, 5)),
				AskQty:   asI64(u(f, 6)),
				AskCount: uint32(u(f, 7)),
				TsNs:     u(f, 8),
				Seq:      u(f, 9),
			}, nil
		case 2: // Snapshot
			bids, err := levels(f, 2)
			if err != nil {
				return nil, err
			}
			asks, err := levels(f, 3)
			if err != nil {
				return nil, err
			}
			return Snapshot{
				SymbolID: uint32(u(f, 1)),
				Bids:     bids,
				Asks:     asks,
				TsNs:     u(f, 4),
				Seq:      u(f, 5),
			}, nil
		case 3: // Delta
			return Delta{
				SymbolID: uint32(u(f, 1)),
				Side:     uint32(u(f, 2)),
				Px:       asI64(u(f, 3)),
				Qty:      asI64(u(f, 4)),
				Count:    uint32(u(f, 5)),
				TsNs:     u(f, 6),
				Seq:      u(f, 7),
			}, nil
		case 4: // Trade
			return MdTrade{
				SymbolID:  uint32(u(f, 1)),
				Px:        asI64(u(f, 2)),
				Qty:       asI64(u(f, 3)),
				TakerSide: uint32(u(f, 4)),
				TsNs:      u(f, 5),
				Seq:       u(f, 6),
			}, nil
		case 5: // Heartbeat
			return MdHeartbeat{TsMs: u(f, 1)}, nil
```

Level is `{px=1, qty=2, count=3}`. Delta side 0 bid / 1 ask, qty 0 removes the
level; Trade taker_side 0 buy / 1 sell; `seq` is the engine height per symbol
(monotonic, non-strict). Golden byte vectors pin the decoder
[code rsx-core/wire/md_test.go TestDecodeMd*GoldenBytes]; glass's mock encoder
is held to the same bytes [code rsx-glass/mockx/contract_test.go].

### Folds

| Fold | Rule | Source |
|---|---|---|
| snapshot | replace both sides wholesale; wire order trusted (bids descending, asks ascending) | [code rsx-core/book/book.go:25-30] |
| delta | insert / replace / remove one level keeping sort order; unknown side ignored | [code rsx-core/book/book.go:38-93] |
| bbo | recorded; used for the mid only when a ladder side is missing and both BBO sides are set (px > 0) | [code rsx-core/book/book.go:124-138] |
| trade | prepended to the tape (cap 50) and queued for the next heatmap bin | [code rsx-core/book/tape.go], [code rsx-term/ui/update.go:661-666] |
| persistence | a level's clock starts when its price first shows size and resets only when it empties | [code rsx-core/book/persist.go] |
| mid | `(best bid + best ask) / 2` from the ladder; else from the BBO if both sides set; never from one side | [code rsx-core/book/book.go:124-138] |

```go
// rsx-core/book/book.go:128-138 (exact)
func (b *Book) Mid() (int64, bool) {
	bid, bidOk := b.BestBid()
	ask, askOk := b.BestAsk()
	if bidOk && askOk {
		return (bid.Px + ask.Px) / 2, true
	}
	if b.bbo != nil && b.bbo.BidPx > 0 && b.bbo.AskPx > 0 {
		return (b.bbo.BidPx + b.bbo.AskPx) / 2, true
	}
	return 0, false
}
```

### Requirements the terminal does not meet

| ID | Requirement | Source |
|---|---|---|
| MD-10 | **Sequence gaps.** On a seq jump > 1 for a symbol, glass re-subscribes that symbol for a fresh snapshot and marks the book stale until it lands. The terminal tracks seq (`SeqTracker.Observe` returns the gap) but ignores the result. | [doc specs/2/49-webproto.md "Gap detection: if seq jumps by more than 1, re-subscribe"], [code rsx-term/ui/update.go:50-66], [code rsx-core/book/seq.go] |
| MD-11 | **Staleness at the point of reading.** When no frame has arrived for 2 s the book region is flagged stale (Degraded) on every screen that shows it, not only in the trace HUD; the mid and touch dim with `~` while stale. | [code rsx-term/ui/view.go:32-34, 664-676], [doc rsx-glass/SPEC.md "Mid-session staleness"] |
| MD-12 | **Crossed book.** A best bid ≥ best ask (a lagging shadow book, a missed delta) is shown as crossed (Degraded), never silently drawn as a normal spread; it triggers MD-10's re-subscribe. | [doc CLAUDE.md "Correctness Invariants" 6], [BUGS RISK-CMP-DESYNC-ORDER-EVENT-STARVE "crossed/stale books at the terminal"], [doc rsx-term/demo/CLAUDE.md "Check the terminal's touch ladder is uncrossed"] |

### Market-data age

Age = client wall clock − the frame's server `ts_ns`, per frame; a frame with
`ts_ns == 0` is unmeasurable and leaves the age unknown; a negative age clamps
to 0; arrival time is recorded either way for staleness. Rolling p50/p99/best
over the same 128-sample window as RTT [code rsx-term/ui/update.go:782-802].
Applied to primary-symbol frames only at the pin (the RSX feed).

## Symbol configuration (gateway REST)

`GET <http(s) sibling of the gateway WS URL>/v1/symbols` →
`{"symbols":[{"id","tick_size","lot_size","price_decimals","qty_decimals"}]}`,
2 s timeout, fetched once at startup and shared by the precision lookup and
the watchlist [code rsx-term/conn/symbols.go], [code rsx-term/main.go:430-444].

| ID | Requirement | Source |
|---|---|---|
| MD-20 | Display precision and tick come from `/v1/symbols`; env overrides (`RSX_TUI_PRICE_DECIMALS`, `RSX_TUI_QTY_DECIMALS`, `RSX_TUI_TICK`) win; a failed fetch falls back to defaults with a stderr note and never blocks startup. | [code rsx-term/main.go:520-540] |
| MD-21 | The endpoint carries no names; names and switcher codes come from config (`id:name[:code]` list); an unnamed id shows `SYM-<id>`; the primary leads the watchlist. | [code rsx-term/main.go:456-518] |
| MD-22 | The gateway serves boot-time tick/lot and hard-codes 8 decimals for every symbol, so the precision it reports can be wrong; the demo overrides decimals by env. Glass shows the precision source in the trace view and accepts the override. | [BUGS GW-SYMBOL-CONFIG-FROM-ENV], [code rsx-term/demo/walk.sh "RSX_TUI_*_DECIMALS"] |
| MD-23 | Lot size is known (`lot_size`) and order quantities are lot-aligned before submit; the gateway rejects off-lot qty with `E[1009]`. The terminal fetches `lot_size` and never uses it. | [code rsx-term/conn/symbols.go:19-26], [code rsx-gateway/src/handler.rs 1009] |

## Hyperliquid (read-only)

| Item | Contract | Source |
|---|---|---|
| universe | `POST https://api.hyperliquid.xyz/info {"type":"meta"}` → `universe[{name, szDecimals, onlyIsolated}]`; id = index + 1; qty decimals = `szDecimals`; price decimals = `max(0, 6 − szDecimals)`; tick 1 raw unit | [code rsx-term/conn/hyperliquid.go:46-107] |
| stream | `wss://api.hyperliquid.xyz/ws`; per coin subscribe `{"method":"subscribe","subscription":{"type":"l2Book","coin":"<c>"}}` and the same with `"trades"`; ping `{"method":"ping"}` every 30 s | [code rsx-term/conn/hyperliquid.go:260-298] |
| l2Book | `{coin, time(ms), levels:[[bids],[asks]]}` each level `{px:"str", sz:"str", n}` → a full `Snapshot` (ts = time × 1e6, seq = time); unparseable or non-positive levels skipped | [code rsx-term/conn/hyperliquid.go:143-155, 332-348, 394-406] |
| trades | array of `{coin, side "B"/"A", px, sz, time, tid}`; `B` → taker buy, `A` → taker sell, any other side SKIPPED | [code rsx-term/conn/hyperliquid.go:349-388] |
| decimal parse | integer-only `ParseHLFixed(s, dec)`: excess fraction truncates, any non-digit or overflow rejects | [code rsx-term/conn/hyperliquid.go:109-141] |
| delivery | non-blocking emit into a 256 buffer: on overflow md drops (l2Book frames are full snapshots, so the next one repairs) | [code rsx-term/conn/hyperliquid.go:313-321] |
| watch set | default 24 coins `BTC ETH SOL DOGE XRP AVAX LINK OP ARB SUI APT SEI TIA WIF PEPE BNB ADA LTC NEAR INJ AAVE CRV LDO JUP`; configurable; `all` = whole universe | [code rsx-term/conn/hyperliquid.go:408-414] |
| sectors | static table → `majors L1 L2 defi meme ai other` | [code rsx-term/conn/hyperliquid.go:416-436] |
| trading | not wired: needs EIP-712 order signing with an ETH key and nonce management | [code rsx-term/conn/hyperliquid.go:9-13] |

```go
// rsx-term/conn/hyperliquid.go:419-435 (exact)
	switch strings.ToUpper(coin) {
	case "BTC", "ETH", "BNB", "SOL", "XRP", "DOGE", "ADA", "LTC":
		return "majors"
	case "AVAX", "NEAR", "APT", "SUI", "SEI", "TIA", "TON", "TRX", "DOT", "ATOM", "INJ":
		return "L1"
	case "OP", "ARB", "STRK", "ZK", "MATIC", "POL", "MANTA", "BLAST":
		return "L2"
	case "AAVE", "CRV", "LDO", "UNI", "MKR", "COMP", "SNX", "SUSHI", "JUP", "PENDLE", "ENA":
		return "defi"
	case "WIF", "PEPE", "BONK", "SHIB", "FLOKI", "MEME", "POPCAT", "BRETT", "MOG", "PENGU":
		return "meme"
	case "FET", "RNDR", "TAO", "WLD", "AI16Z", "VIRTUAL":
		return "ai"
	default:
		return "other"
	}
```

## Phoenix (read-only, orderbook only)

| Item | Contract | Source |
|---|---|---|
| universe | `GET https://perp-api.phoenix.trade/v1/view/exchange/markets` → bare array `[{symbol, tickSize, baseLotsDecimals, assetId, …}]`; id = assetId + 1; price decimals fixed 4; qty decimals = baseLotsDecimals clamped to [0, 8] | [code rsx-term/conn/phoenix.go:43-130] |
| stream | `wss://perp-api.phoenix.trade/v1/ws`; per symbol `{"type":"subscribe","subscription":{"channel":"orderbook","symbol":"<s>"}}`; ping `{"type":"ping"}` every 30 s | [code rsx-term/conn/phoenix.go:239-276] |
| orderbook | `{channel:"orderbook", symbol, orderbook:{bids:[[px,sz]], asks:[[px,sz]], mid}}` with JSON numbers read as decimal strings (no float) → a full `Snapshot`; no per-level count (0) | [code rsx-term/conn/phoenix.go:132-151, 305-343] |
| trades | not decoded: prints carry `quoteAmount/baseAmount`, not a price; deriving a fixed-point price needs a scaled division that overflows i64 for large notionals | [code rsx-term/conn/phoenix.go:15-21] |
| watch set | default `SOL BTC ETH XRP HYPE DOGE BNB SUI JUP WLD`; `all` = every market | [code rsx-term/conn/phoenix.go:345-350] |
| trading | not wired: on-chain settlement needs a Solana wallet signer | [code rsx-term/conn/phoenix.go:9-13] |

Phoenix snapshots carry no `ts_ns` and no seq, so their age is unknown (H-11).

## Offline mock

The default path. A recording Submitter (records every submit and cancel; a
`Down` flag makes both fail with `gateway link down`) plus a scripted feed
paced at 30 ms per message [code rsx-term/conn/mock.go], [code rsx-term/main.go:624-631].
The script: link up, a 4×4 PENGU book, two prints, an own order accepted
(oid 7, RTT 10.44 µs), filled 14 @ 9998 and done, two full-split latency
samples (net 2.5 / internal 7.6 / engine 0.34 µs and 2.3 / 7.1 / 0.31 µs), then a
SOL-PERP peer book and two prints. It derives the position from a real own
order lifecycle, not a synthetic position event
[code rsx-term/conn/mock.go:70-129]. The mock is the proof the seam is real
[doc rsx-term/notes/venue-seam.md "The mock is the proof"].

MD-30: glass's offline path covers both channels — market data (glass has
`mockx`) and the order channel (glass has none) — so every flow in
[11-flows.md](11-flows.md) runs headless with no network.

## News (Tree of Alpha)

| Item | Contract | Source |
|---|---|---|
| opt-in | off by default; one env flag starts the reader; the only dial site | [code rsx-term/main.go:257-267] |
| stream | `wss://news.treeofalpha.com/ws`; JSON objects `{title, body, source, symbols[], time(ms), …}`; a frame with no title and no body, or no time, is not a headline (pings, receipts) | [code rsx-term/news/treeofalpha.go:13-168] |
| buffer | in-memory ring of 512, newest last; reads never touch the network, so rendering never blocks on news | [code rsx-term/news/treeofalpha.go:21-95] |
| reconnect | backoff 1 s doubling to 60 s, silent by design (news is an overlay, never a dependency) | [code rsx-term/news/treeofalpha.go:24-115] |
| tags | upper-cased raw symbol tags (`BTC`, `BTCUSDT`); linking to an instrument is an exact-name or base-prefix match upstream | [code rsx-term/news/treeofalpha.go:153-160], [code rsx-term/ui/news_view.go:495-512] |

Severity is a keyword heuristic (Tree of Alpha has no severity field), a
triage cue, not a verdict:

```go
// rsx-term/news/treeofalpha.go:175-204 (exact)
func DeriveSeverity(text string, symbols []string) int {
	lower := strings.ToLower(text)
	for _, w := range criticalWords {
		if strings.Contains(lower, w) {
			return 3
		}
	}
	for _, w := range highWords {
		if strings.Contains(lower, w) {
			return 2
		}
	}
	if len(symbols) > 0 {
		return 1
	}
	return 0
}

var criticalWords = []string{
	"hack", "exploit", "halt", "bankrupt", "insolven", "depeg",
	"emergency", "attack", "drained", "rug", "seized",
}

var highWords = []string{
	"sec ", "etf", "fed ", "rate", "lawsuit", "sues", "delist",
	"listing", "lists", "acquisition", "acquire", "partnership",
	"upgrade", "fork", "liquidat",
}
```

The `news.Source` interface is the seam for other headline sources:
`Markers(sinceNs, untilNs)`, `All()` newest first, `Enabled()`; the default is
an always-empty source [code rsx-term/news/news.go]. Headlines are
attacker-controlled text; they reach the assistant's context, which is why
the assistant's egress is an open safety question
([14-contract-support-assistant.md](14-contract-support-assistant.md)).
