---
status: draft
---

# 10 — Keymap

The keymap is the complete set of keys the trading UI answers to, on every
screen and in every capture mode, plus the rules that keep dispatch, the hint
line, the help overlay and the keycast label generated from one table. Flows
that string these keys together are [11-flows.md](11-flows.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Rules

| ID | Requirement | Source |
|---|---|---|
| KM-01 | Keyboard-only. No mouse input, ever; there is no click-to-price. | [code rsx-term commit 92bf9817 "remove mouse input entirely"], [doc rsx-term/CLAUDE.md "Keeper invariants"], [founder 2026-10-21 "Keyboard-only and every key earns its place"] |
| KM-02 | ONE table drives dispatch, the per-screen hint line, the `?` help overlay and the keycast label; help text can never drift from the binding. | [code rsx-term/ui/keymap.go:9-18], [founder 2026-10-21 "the keymap is the one source of truth for help text"] |
| KM-03 | `?` opens help on every screen; help is generated per screen from the table. | [code rsx-term/ui/keymap.go:310-342], [founder 2026-10-21 "`?` help on every screen"] |
| KM-04 | Verb keys are rebindable from a JSON `{"action":"key"}` file. Key CLASSES (digits as presets, shifted digits as crosses, letters as symbol selectors, typed text) are structural and fixed. | [code rsx-term/ui/keymap.go:11-14] |
| KM-05 | A rebind naming an unknown action, an empty key, or a key already bound on the same screen (screen table plus globals) rejects the WHOLE map: defaults stay active and the status line says `KEYMAP REJECTED: <err> — defaults active`. A broken map never half-applies. An unreadable or non-JSON file warns on stderr and keeps defaults. | [code rsx-term/ui/keymap.go:235-277], [code rsx-term/ui/model.go:414-421], [code rsx-term/main.go:236-255] |
| KM-06 | The hint line shows a rebound key as bound (the hint's leading key token is replaced). | [code rsx-term/ui/keymap.go:296-305] |
| KM-07 | Capital-at-risk keys are flagged `danger` and drawn in the Ask hue in help. | [code rsx-term/ui/keymap.go:76], [code rsx-term/ui/view.go:354-393] |
| KM-08 | Prefix keys (`x`, `F9`, `/`) open a one-shot capture mode instead of a global leader namespace: flat per-screen maps with modal prefixes. | [code rsx-term/ui/keymap.go:14-18] |
| KM-09 | In any capture mode every key is input, including `q`; only the mode's own exit keys leave it. | [code rsx-term/ui/update.go:193-219], [code rsx-term/ui/keymap_test.go TestSearchCapturesGlobalKeys] |
| KM-10 | Orders on the BOOK screen fire on ONE key; the size and notional caps still hard-block; the help header says so. | [code rsx-term/ui/update.go:188-192], [code rsx-term/ui/keymap.go:318-323] |

## The table (streaming screens)

```go
// rsx-term/ui/keymap.go:103-165 (exact)
	return &keymap{
		global: []binding{
			{action: actQuit, key: "q", alts: []string{"ctrl+c"}, help: "quit", hint: "q quit"},
			{action: actNextView, key: "tab", help: "next view (book → news → assistant → users)", hint: "tab view"},
			{action: actPrevView, key: "shift+tab", help: "previous view"},
			{action: actVenuePick, key: "f9", help: "venue picker (then the venue's letter)"},
			{action: actReduceOnly, key: "r", help: "toggle reduce-only (applies to every order)", hint: "r RO"},
			{action: actPostOnly, key: "p", help: "toggle post-only (applies to resting orders)"},
			{action: actHelp, key: "?", help: "this help", hint: "? help"},
		},
		book: []binding{
			{action: actSwitchSymbol, key: "x", help: "symbol switcher (then the symbol's code)", hint: "x symbol"},
			{action: actOpenNews, key: "n", help: "news view", hint: "n news"},
			{action: actBuySide, key: "b", help: "side: buy", hint: "b/s side"},
			{action: actSellSide, key: "s", help: "side: sell"},
			{action: actCursorDown, key: "h", alts: []string{"left"}, help: "cursor one tick down", hint: "h/l cursor"},
			{action: actCursorUp, key: "l", alts: []string{"right"}, help: "cursor one tick up"},
			{action: actCursorBid, key: "j", help: "cursor to the best bid", hint: "j/k touch"},
			{action: actCursorAsk, key: "k", help: "cursor to the best ask"},
			{action: actRowUp, key: "up", help: "microscope: cursor to an older row (freeze, not replay)"},
			{action: actRowDown, key: "down", help: "microscope: cursor to a newer row"},
			{action: actFreeze, key: "enter", help: "freeze the cursor row → assistant", danger: false},
			{action: actPlace, key: "f", help: "place resting limit at the cursor", hint: "f place", danger: true},
			{action: actCancel, key: "d", help: "cancel own order nearest the cursor", hint: "d cancel", danger: true},
			{action: actQuitBook, key: "esc", help: "quit (esc again if the microscope is on turns it off first)"},
		},
		news: []binding{
			{action: actSearch, key: "/", help: "search the feed (type, enter keeps, esc clears)", hint: "/ search"},
			{action: actFeedDown, key: "j", alts: []string{"down"}, help: "select next headline", hint: "j/k select"},
			{action: actFeedUp, key: "k", alts: []string{"up"}, help: "select previous headline"},
			{action: actHandoff, key: "enter", help: "hand the headline + frozen book to the assistant", hint: "enter → assistant"},
			{action: actBack, key: "esc", help: "clear search / back to the book", hint: "esc back"},
		},
		llm: []binding{
			{action: actBack, key: "esc", help: "back to the news view", hint: "esc → news"},
		},
		users: []binding{
			{action: actUsersSearch, key: "/", help: "search the directory (list mode)", hint: "/ search"},
			{action: actUsersDown, key: "j", alts: []string{"down"}, help: "select next user (list mode)", hint: "j/k select"},
			{action: actUsersUp, key: "k", alts: []string{"up"}, help: "select previous user (list mode)"},
			{action: actUsersOpen, key: "enter", help: "open the selected user's detail", hint: "enter detail"},
			{action: actUsersRefresh, key: "g", help: "refresh the list, or the open detail", hint: "g refresh"},
			{action: actUsersSetRole, key: "o", help: "set role (admin-only server-side)", hint: "o role", danger: true},
			{action: actUsersAdjustCollateral, key: "a", help: "adjust collateral (admin-only server-side)", hint: "a adjust", danger: true},
			{action: actUsersFreezeUpsert, key: "z", help: "freeze an order (admin-only server-side)", hint: "z freeze", danger: true},
			{action: actUsersFreezeDelete, key: "Z", help: "unfreeze an order (admin-only server-side)", danger: true},
			{action: actUsersAddNote, key: "n", help: "add a note (operator+ server-side)"},
			{action: actUsersBack, key: "esc", help: "detail → list → book view", hint: "esc back"},
		},
		bookClasses: []classDoc{
			{keys: "1-5", help: "arm a size preset"},
			{keys: "⇧1-5", help: "cross NOW — IOC at the far touch, preset size", danger: true},
		},
		newsClasses: []classDoc{
			{keys: "a-z", help: "jump into the symbol's book view"},
		},
		llmClasses: []classDoc{
			{keys: "type", help: "type to chat · enter sends · esc backs out (when the assistant is wired via RSX_TERM_ASSIST)"},
		},
		usersClasses: []classDoc{
			{keys: "type", help: "an open action prompt captures typing · enter submits · esc cancels"},
		},
	}
```

Rebindable action ids (the JSON keys): `back buy-side cancel cursor-ask
cursor-bid cursor-down cursor-up feed-down feed-up freeze handoff help
next-view open-news place post-only prev-view quit quit-book reduce-only
row-down row-up search sell-side switch-symbol users-add-note
users-adjust-collateral users-back users-down users-freeze-delete
users-freeze-upsert users-open users-refresh users-search users-set-role
users-up venue-pick` [code rsx-term/ui/keymap.go:20-65, 351-367]. A rebind
of an action that appears on several screens (`back`) rebinds it everywhere
[code rsx-term/ui/keymap.go:239-258].

### Fixed key classes (BOOK)

| Keys | Action | Source |
|---|---|---|
| `1`–`5` | arm size preset 1–5 (status `size <qty> armed`) | [code rsx-term/ui/update.go:301-303] |
| `!` `@` `#` `$` `%` (shift+1–5) | cross NOW: arm that preset AND fire an IOC at the far touch (buy lifts best ask, sell hits best bid); the preset changes only if the cross fires | [code rsx-term/ui/update.go:304-305, 501-525] |
| `a`–`z` (NEWS) | the symbol whose code it is opens in BOOK on the breadth venue | [code rsx-term/ui/news_view.go:375-382] |

Default size presets: 1, 2, 5, 10, 25 whole units at the active instrument's
qty precision, so a symbol hop keeps presets meaningful
[code rsx-term/ui/model.go:83-100].

### Symbol codes (the `x` switcher and NEWS letters)

```go
// rsx-term/ui/market.go:70-120 (exact, comments trimmed)
const letterPool = "afghjkl" + "wetyuio" + "zcvnm"

func assignCodes(instruments []Instrument) {
	used := map[string]bool{}
	for i := range instruments {
		if c := strings.ToLower(instruments[i].Code); c != "" {
			instruments[i].Code = c
			used[c] = true
		}
	}
	for i := range instruments {
		if instruments[i].Code != "" {
			continue
		}
		instruments[i].Code = pickCode(instruments[i].Name, used)
		used[instruments[i].Code] = true
	}
}

func pickCode(name string, used map[string]bool) string {
	lower := strings.ToLower(name)
	for _, r := range lower {
		if strings.ContainsRune(letterPool, r) && !used[string(r)] {
			return string(r)
		}
	}
	for _, r := range letterPool {
		if !used[string(r)] {
			return string(r)
		}
	}
	for _, a := range letterPool {
		for _, b := range letterPool {
			code := string(a) + string(b)
			if !used[code] {
				return code
			}
		}
	}
	return "??" // >600 symbols: beyond the terminal's design point
}
```

The pool excludes the verb keys `b s d r p q x` and digits. Codes are assigned
per venue in list order; an explicit code (`RSX_TERM_WATCH id:name:code`) is
kept and reserved first [code rsx-term/main.go:497-518]. Note: the pool
contains `j`, `k`, `n`, `a`, `z`, `g`, `o` — on NEWS `j`/`k` are bound verbs, so
a symbol coded `j` or `k` is unreachable by letter on NEWS (only via `x` on
BOOK). Requirement KM-11: glass assigns codes so that no code collides with a
verb bound on any screen where the code is typed outside a capture mode
(defect in the terminal; listed in [20-rsx-term-record.md](20-rsx-term-record.md)).

## Dispatch precedence (streaming)

Order of checks for every key [code rsx-term/ui/update.go:193-258]:

1. Help overlay open: `q`/`ctrl+c` quit; any other key closes help.
2. Symbol switcher open → switcher grammar.
3. Venue picker open → picker grammar.
4. NEWS search typing → search grammar (captures `q`).
5. LLM screen with a live assistant, key not `tab`/`shift+tab` → chat input
   grammar (captures `q`, `?`, `r`, `p`; `ctrl+c` still quits).
6. USERS with an open action prompt → prompt grammar (captures `q`).
7. USERS search typing → search grammar.
8. The table: screen bindings first, then globals.
9. Globals handled: quit, help, next/prev view (entering USERS triggers its
   first load), venue picker (opens only with more than one venue, else status
   `one venue configured (<name>)`), RO and PO toggles (status
   `reduce-only on (applies to every order until toggled)`).
10. Screen grammar, then fixed classes.

### Capture-mode grammars

| Mode | Opened by | Keys | Source |
|---|---|---|---|
| symbol switcher | `x` (BOOK) | `a`–`z` append to the buffer; an exact code match switches instantly; a 2-letter miss reports `no symbol coded "<buf>"` and closes; `backspace` edits; `esc` or `x` cancels | [code rsx-term/ui/update.go:311-336] |
| venue picker | `F9` | the venue's letter (its first letter, lower case) switches; `esc`/`F9` cancels; any other key closes with `no venue keyed "<k>"` | [code rsx-term/ui/update.go:364-388], [code rsx-term/ui/model.go:455-461] |
| news search | `/` (NEWS) | printable ASCII appends (cap 48) and resets the selection; `backspace`; `enter` keeps the filter; `esc` clears it | [code rsx-term/ui/news_view.go:387-407] |
| chat input | live assistant on LLM | printable ASCII appends (cap 512); `backspace`; `enter` sends on the current thread (refused while a turn is in flight: draft kept, status `assistant is busy — wait for the reply`); `esc` clears a draft, else goes to NEWS; `tab`/`shift+tab` still cycle | [code rsx-term/ui/news_view.go:713-756] |
| users search | `/` (USERS list) | as news search, over login and email | [code rsx-term/ui/users_view.go:461-481] |
| users action prompt | `o a z Z n` (USERS detail) | printable ASCII appends (cap 200); `backspace`; `enter` parses and submits (a malformed line keeps the prompt open with the reason); `esc` cancels without sending | [code rsx-term/ui/users_view.go:483-564] |

### Screen grammar notes

| Screen | Key | Behaviour | Source |
|---|---|---|---|
| BOOK | `esc` | microscope on → turn it off (status `microscope off`); else quit | [code rsx-term/ui/update.go:265-271] |
| BOOK | `up` / `down` | first press enters the microscope at the newest held row; then older / newer, clamped | [code rsx-term/ui/update.go:440-456] |
| BOOK | `enter` | with a microscope row: freeze it → LLM; else status `microscope: move the row cursor first (↑/↓)` | [code rsx-term/ui/news_view.go:451-469] |
| BOOK | `h` / `l` | cursor ±1 tick, seeded from the mid rounded down to the tick (else one tick), floored at one tick | [code rsx-term/ui/update.go:419-438] |
| BOOK | `j` / `k` | cursor to best bid / best ask; empty side → status `no bid to join` / `no ask to join` | [code rsx-term/ui/update.go:458-474] |
| BOOK | `f` | GTC limit at the cursor; no cursor → the side's own touch; empty touch → `place: no bid to join (move the cursor first)` | [code rsx-term/ui/update.go:476-499] |
| BOOK | `d` | cancel the own order on the active symbol nearest the cursor; no cursor → the newest; none → `no open order to cancel` | [code rsx-term/ui/update.go:527-560] |
| NEWS | `esc` | non-empty query → clear it; else BOOK | [code rsx-term/ui/news_view.go:361-366] |
| LLM (offline) | `esc` | NEWS | [code rsx-term/ui/news_view.go:706-711] |
| USERS | `esc` | detail → list → (clear query) → BOOK, one layer per press | [code rsx-term/ui/users_view.go:377-387] |
| USERS | `/ j k` | list mode only | [code rsx-term/ui/users_view.go:388-402] |
| USERS | `o a z Z n` | detail mode only; open a prompt | [code rsx-term/ui/users_view.go:407-416] |

The screen cycle is BOOK → NEWS → LLM → USERS → BOOK (`tab`), reverse on
`shift+tab` [code rsx-term/ui/model.go:345-375]. The global help text for
`tab` reads "next view (book → news → assistant → users)".

## Classic order-entry view keys

The classic three-column view dispatches a hard-coded switch, not the table
[code rsx-term/ui/update.go:836-936]; its legend is a static string
[code rsx-term/ui/view.go:38-40]:

```
 q quit  b/s side  t tif  r ro  p po  +/- tick  j/k join  tab field  0-9 type  ⌫ del  enter submit  m mkt  ↑↓ sel  c cancel  X all  x flatten  R reverse  F2 armed  F3 trace  ? help
```

| Key | Action | Danger | Source |
|---|---|---|---|
| `q` / `ctrl+c` | quit | yes | [code rsx-term/ui/update.go:851-852] |
| `?` | help overlay (any key closes; `q` still quits) | | [code rsx-term/ui/update.go:843-855] |
| `esc` | cancel a pending confirm (`order not sent`), else quit | | [code rsx-term/ui/update.go:856-862] |
| `enter` | first press: confirm preview; second: submit. ARMED: submit at once | | [code rsx-term/ui/update.go:1038-1063] |
| `0`–`9`, `.` | type into the focused field (cap 18 chars; one dot; leading dot → `0.`; ignored at 0 decimals) | | [code rsx-term/ui/update.go:995-1023] |
| `backspace` | delete the last character | | [code rsx-term/ui/update.go:1025-1036] |
| `tab` | price ⇄ qty focus | | [code rsx-term/ui/update.go:905-910] |
| `b` / `s` | side | | |
| `t` | TIF GTC → IOC → FOK → GTC | | [code rsx-core/wire/types.go:47-57] |
| `r` / `p` | reduce-only / post-only toggles | | |
| `+` `=` / `-` | price ±1 tick (seeded from mid, floored at one tick), focuses price | | [code rsx-term/ui/update.go:961-971] |
| `j` / `k` | price = best bid / best ask | | [code rsx-term/ui/update.go:973-993] |
| `m` | market: IOC at the far touch for the qty, through the confirm | | [code rsx-term/ui/update.go:1128-1154] |
| `↑` / `↓` | move the working-order cursor | | |
| `c` | cancel the selected working order | yes | [code rsx-term/ui/update.go:1259-1278] |
| `X` | cancel every working order | yes | [code rsx-term/ui/update.go:1280-1299] |
| `x` | flatten: reduce-only IOC closing the whole position at the opposing touch, through the confirm | yes | [code rsx-term/ui/update.go:1107-1126] |
| `R` | reverse: IOC of 2× net on the opposite side, not reduce-only, through the confirm | yes | [code rsx-term/ui/update.go:1188-1218] |
| `F2` | ARMED toggle (confirm off; banner) — clears any pending confirm | yes | [code rsx-term/ui/update.go:884-892] |
| `F3` | trace HUD toggle | | [code rsx-term/ui/update.go:881-883] |

Every editing key clears a pending confirm (its preview would be stale)
[code rsx-term/ui/update.go:895-935].

**Conflict across views.** `x` means "symbol switcher" on BOOK and "flatten"
in the classic view; `esc` quits from BOOK and from the classic view. Glass has
one keymap; how the classic view's verbs (typed price, TIF cycle, market,
flatten, reverse, ARMED, cancel-all, cancel-selected) join the table is OD-03
in [16-open-decisions.md](16-open-decisions.md).

## Keycast labels

The keycast chip labels a key with the `help` of the binding that dispatches
it on the active screen, or the class doc for `1-5`, `⇧1-5`, `a-z`; the label
drops a trailing parenthetical and caps at 40 runes. In a capture mode (help,
switcher, picker, news search, live chat) the chip shows the bare key. The
classic view never labels (it does not dispatch through the table).
[code rsx-term/ui/keycast.go:50-115]. Screen placement:
[09-chrome.md](09-chrome.md).

## Keys the founder asked for that the terminal lacks

| ID | Requirement | Source |
|---|---|---|
| KM-20 | A chat key on every screen opens the assistant with the vantage of the screen it was pressed on (one new table row, so it lands in hints, help and rebinds). | [doc specs/2/60-terminal-assistant.md "UX — chat key and key legend"], [founder 2026-10-21 "The assistant is a chat on every screen"] |
| KM-21 | Every screen renders a compact key legend (top-right) generated from the same table's hint fields. | [doc specs/2/60-terminal-assistant.md "Key legend, every screen, top-right"] |
| KM-22 | `?` help exists on every screen, including the classic order-entry view, generated from the table. | [founder 2026-10-21 "`?` help on every screen"]; the terminal's classic help is hand-written [code rsx-term/ui/view.go:354-393] |

Key choices for KM-20 are open (OD-04).
