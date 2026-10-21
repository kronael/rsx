---
status: draft
---

# 08 — Screen: USERS (users and wallets)

USERS is the management screen: the identity and role directory from
`rsx-auth`, one user's account, positions, fills, funding and order freezes
from the gateway's support API, and single-line action prompts to set a role,
adjust collateral, freeze or unfreeze an order, or add a note — every write
going through the real client and showing the server's real answer. User and
wallet management belong in the native UI, never on the web
[founder F-SURF-03, F-SCR-08]. Contracts:
[14-contract-support-assistant.md](14-contract-support-assistant.md); keys:
[10-keymap.md](10-keymap.md); acceptance: [17-acceptance.md](17-acceptance.md) § USERS.

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.
Frames below were rendered from the pinned code (fixture roles: alice admin,
bob no role; fixture accounts; 100 × 32; blank rows collapsed)
[code rsx-term/ui/users_view_test.go helpers].

## Layout

List:

```
 RSX  PENGU-PERP   ● offline  ◀ bids  mid —  asks ▶
 USERS  rsx  RO off  PO off  size 1.0000 [1]  list rsx
 ── users ── / search · j/k select · enter detail · g refresh · you: admin ──
  id     login                email                    role
▸ 1      alice                alice@rsx.dev            admin
  2      bob                  bob@rsx.dev              —
…
 connecting…
 / search  j/k select  enter detail  g refresh  o role  a adjust  z freeze  esc back  q quit  tab view  r RO  ? help
```

Detail, then an open prompt (the prompt replaces the status line):

```
  user 1 — alice
  role admin   email alice@rsx.dev
  ── account (raw units) ──────────────────
  collateral 100000000  equity 104500000  uPnL 4500000  avail 88000000
  ── positions (1) ──────────────────────
  sym 10  long qty 5000000 @ 9950000  uPnL 4500000
  ── freezes (0) ─────────────────────────

  o set-role · a adjust-collateral · z freeze · Z unfreeze · n note · esc back
…
 adjust collateral — <delta> <ticket_id> <reason…>: 500 T-1 fix_
```

## Elements and states

| Element / state | Content | Source |
|---|---|---|
| list header | ` ── users ── / search · j/k select · enter detail · g refresh · you: <role> ──`; `you: —` with no role; `you: — (rsx-auth not configured)` offline; while searching `── search: <q>_ ──` | [code rsx-term/ui/users_view.go:212-230] |
| identity offline | `  rsx-auth not configured — set RSX_TERM_AUTH_URL + RSX_TERM_SUPPORT_TOKEN` | [code rsx-term/ui/users_view.go:236-239] |
| loading | `  loading…` (list) / `  loading account/positions/fills/funding/freezes…` (detail) | [code rsx-term/ui/users_view.go:240-243, 299-302] |
| error | `  error: <server error>` in Ask (list or detail) | [code rsx-term/ui/users_view.go:244-247, 303-305] |
| empty | `  no users` | [code rsx-term/ui/users_view.go:248-252] |
| list rows | `id login email role` columns (6 / 20 / 24 / 10, truncated with `…`), role `—` when none, `▸` on the selection; `… N more` past the visible rows | [code rsx-term/ui/users_view.go:253-272] |
| detail head | `user <id> — <login>`; `role <role>   email <email>` (dashes when empty) | [code rsx-term/ui/users_view.go:295-298] |
| accounts not configured | derived `~ account/positions/fills/funding/freezes: gateway support surface not shipped yet (stub) —` and a pointer line | [code rsx-term/ui/users_view.go:306-309] |
| account block | `── account (raw units) ──`; `collateral … equity … uPnL … avail …` as raw i64 (an account spans symbols with different precisions; guessing one would fabricate) | [code rsx-term/ui/users_view.go:290-337] |
| positions | `── positions (N) ──`; `sym <id>  long|short qty <q> @ <entry>  uPnL <u>` | [code rsx-term/ui/users_view.go:324-331] |
| freezes | `── freezes (N) ──`; `order <id> — <reason> (<ticket>)` in Ask | [code rsx-term/ui/users_view.go:332-335] |
| action legend | `o set-role · a adjust-collateral · z freeze · Z unfreeze · n note · esc back` | [code rsx-term/ui/users_view.go:312] |
| prompt | `<title>: <input>_` in the status line, titles below | [code rsx-term/ui/users_view.go:45-59, 202-210] |
| result | status `<summary> ok` or `<summary> failed: <server error>`; a successful write refreshes the detail | [code rsx-term/ui/update.go:161-169] |

Prompts and their parse rules (a malformed line keeps the prompt open with the
reason; no default is ever guessed) [code rsx-term/ui/users_view.go:505-564]:

| Key | Title | Grammar | Rejections |
|---|---|---|---|
| `a` | `adjust collateral — <delta> <ticket_id> <reason…>` | integer delta, ticket, reason words | `need: <amount> <ticket_id> <reason…>`, `delta must be an integer: <x>` |
| `z` | `freeze order — <order_id> <ticket_id> <reason…>` | u64 order id | `order_id must be an integer: <x>` |
| `Z` | `unfreeze order — <order_id> <ticket_id> <reason…>` | u64 order id | as above |
| `o` | `set role — <viewer|operator|admin|auditor> <ticket_id> <reason…>` | one of the four roles | `role must be one of viewer/operator/admin/auditor, got <x>` |
| `n` | `add note — <ticket_id> <note…>` | ticket, note words | `need: <ticket_id> <note…>` |

Permissions are enforced server-side only: a denied action shows the server's
real 403, never a locally guessed permission; the header shows the caller's
own role so the operator sees what they may do before trying
[code rsx-term/ui/users_view.go:370-375, 212-214]. The first entry to the
screen loads the list and the caller's role once; `g` refreshes the list or
the open detail [code rsx-term/ui/users_view.go:358-368, 440-449]. `esc` backs
out one layer per press: prompt → detail → (clear query) → BOOK.

## Requirements beyond the terminal

| ID | Requirement | Source |
|---|---|---|
| US-01 | Fills and funding are shown in the detail; the terminal fetches 20 of each and renders neither. Position mark and liquidation price, and the account's initial and maintenance margin, are shown (fetched, not rendered). | [code rsx-term/ui/users_view.go:115-133, 316-337], [code rsx-term/support/support.go:24-64] |
| US-02 | A detail with the accounts client offline shows one honest line, not an `error: support: not configured (offline)` row AND the stub note (both render at the pin). | rendered frame at the pin |
| US-03 | Account money renders in human units per symbol where the symbol's precision is known, and raw (labelled `raw units`) where it is not. | [code rsx-term/ui/users_view.go:290-294] |
| US-04 | The hint line shows only the keys that act in the current mode (the list mode hints `o a z` which do nothing there, and never hints `n`). | [code rsx-term/ui/keymap.go:139-151], rendered frame |
| US-05 | Writes carry an `Idempotency-Key` per logical action (the terminal sets one on set-role only) and an `X-Request-Id`. | [code rsx-term/support/http.go SetRole], [code rsx-term/support/http.go "Proposed contract … X-Request-Id, Idempotency-Key"] |
| US-06 | The add-note write sends the note and a reason; at the pin the note text is sent as both `note` and `reason`. | [code rsx-term/ui/users_view.go:172-177] |
| US-07 | The support token is an already-issued `rsx-auth` session JWT, never self-minted (a self-minted token could assert any role through the shared secret). | [code rsx-term/support/http.go HTTPRolesClient doc], [doc .ship/48-USER-WALLET-MGMT/PLAN.md "the terminal must not invent its own privilege"] |
| US-08 | The help overlay fits the window: the rebindable-action list wraps (at the pin it is one line of ~400 columns). | rendered help frame at the pin |
| US-09 | A wallet view: collateral movements and balances per asset, the founder's "walet management" (no wallet surface exists beyond adjust-collateral at the pin; the gateway contract has none). | [founder F-SCR-08], OD-13 |
