# CLAUDE.md — rsx-playground

Local to `rsx-playground/`; the root `../CLAUDE.md` applies. The operator's
dashboard (server-rendered FastAPI + HTMX + Tailwind, dark theme): this file
holds its role, the founder's standing rules and its visual design system.

## Role

The operator's toolset and the exchange's demonstration: start, stop and
inject faults, inspect WAL and logs, manage users and roles, edit symbol
config, show every component working with a link to its docs. NEVER the
trading front; trading is `rsx-glass`. The gateway is the
production piece, the playground demonstrates it.

## Rules

- Real system only: the dashboard, the API tests and the Playwright specs
  drive the real cluster through the real gateway and marketdata. NEVER a
  mock exchange, a simulated order path or a Python reimplementation of
  exchange logic; stress is real orders from the maker/stress bot, managed as
  a process like the others.
- Every component gets a page: what it is, a link to its docs (GUIDE is the
  playground's own docs, DOCS the platform's), its log tail, its live
  counters, and the thing it does (book, fills, WAL records) visible.
- Demos and promos appear on the `/demos` page only, never on component pages.
- Config edits write a new `symbol_config_schedule` row; the active row is
  never updated. The schema is `rsx-risk`'s.
- The e2e latency card is browser → proxy → gateway milliseconds, a demo aid;
  no Playwright spec asserts latency.
- `make gate` is the release check, run whole and in order; a flaky stage is
  fixed, never gated or disabled. Gate 4 (browser) needs a quiet box.
- Relative URLs everywhere; the dashboard runs under a proxy prefix.
- Python HTTP and WebSocket clients use `aiohttp`; NEVER the `websockets`
  package.
- WAL views come from `rsx-cli` JSON (`rsx_cli.py` runs it, `wal_feed.py`
  follows the streams); the playground holds no WAL decoder and no CRC
  logic. A missing `target/debug/rsx-cli` stops the server at startup.
- Orders are placed, cancelled and batched by `rsx-cli order … --json`
  (`order_cli.py` builds the call and reads the outcome); the order routes hold
  no gateway client and no order wire. Every order route is admin-gated, and a
  reject, a timeout, a missing binary or a down gateway reaches the page as
  text, never as a silent success.
- Invariant views come from `rsx-cli verify --json` (`invariants.py` runs it,
  `render_verify` shows each check as PASS, FAIL or SKIP); the playground holds
  no invariant check. A run that cannot complete renders its reason, never a
  pass.
- The exchange's processes are started, stopped, restarted, listed and reset
  by `rsx-cli cluster … --json` (`cluster_cli.py` runs it); they run detached
  and outlive the server. The server holds no spawn plan and no scenario
  table: scenarios come from `cluster scenarios`, the plan from `cluster
  plan`, and the symbols it needs from Postgres. The watcher keeps the
  restart policy (backoff, circuit) and restarts through `cluster restart`.
- Stress runs and latency probes are `rsx-cli stress` (`stress_cli.py` starts
  the child, reads its report; `tmp/stress-reports/stress-<id>.json` is the
  CLI's report object, rendered as it is). At most one load run is a child of
  the server; it ends with the server, and a stop lets the CLI report what it
  sent. The server holds no order generator, no gateway client and no latency
  math for the exchange's numbers.
- Faults are `rsx-cli fault … --json` (`fault_cli.py` runs it): a crash is
  `fault kill`, the loopback netem is `fault net`, the duplicate-order demo is
  `fault dup-order` (a scratch matching engine inside the CLI), and the WAL
  corruption is `fault wal-corrupt <stream> --wal-dir <stream directory>` on
  the live stream written last, never the recorder archive. The server holds
  no signal, `tc` or WAL-byte code; a CLI refusal reaches the page as text.
- The maker starts with every scenario, so books are populated without a
  manual step; an empty book on a running cluster is a bug.
- A busy-spinning daemon at 100% CPU is healthy: a component shows queue
  depth and processed counts, not CPU load. Verify rows show their reason by
  default; there is no hide-detail toggle.
- The playground carries only what the demos need; unused code and pages are
  deleted, not kept.
- Tooling is Python under uv (`pyproject.toml`); the `playground` CLI is the
  one entrypoint (`start`, `start-all`, `stop-all`, `reset`, `ps`, `logs`).

## The one visual rule: SURROUNDING ACCENTS

Callouts, hints, section accents, and typed status blocks use a
**full-ring accent border** (a "ribbon") — never a left-only bar,
never a top banner. This matches krons.fiu.wtf (`.tldr-box` = 2px
all-sides, `.insight` = 1px all-sides), both `border-radius:3px`.

- **ALWAYS** `border-2 border-{c}-500/70 rounded-[3px]` (prominent:
  hints, primary callouts) or `border border-{c}-500/60 rounded-[3px]`
  (subtle: inline rows, secondary notes) on a `bg-slate-800/50` panel.
- **NEVER** left-only bars (`border-l-*`), top-border banners
  (`border-t`), or full-width colored header bars. A surrounding ring
  reads as a framed annotation on the content; a partial edge reads as
  chrome.
- The **border colour carries the meaning** (see palette): the text
  stays neutral slate; the semantic signal is the ring, not a loud
  background.

Reference: the walkthrough hint (`_hint_bar`, `pages.py`) is the
canonical form — `border border-slate-600/60 rounded-[3px]
bg-slate-800/50`, ≤2 sentences, dismissable. Copy that shape for new
callouts.

## Palette + semantics

Dark theme, class-based (`html.dark`, synchronous `<head>` toggle to
avoid flash). Colour is reserved for meaning, used sparingly.

**Palette: Ayam Cemani ("black iridescence").** The base is a
green-tinged near-black; the accents are the bird's beetle-green +
violet feather-sheen. The Tailwind scales are **retuned in one place**
— `pages.py` `tailwind.config` `theme.extend.colors` — so class names
below are unchanged; only the rendered colour moved:
- `slate` → green-tinged near-black ramp (page `#040806`, panel
  `#0d1712`, borders `#16211b`, body text `#a9bcb2`)
- `emerald` → neon beetle-green `#22f5a1` (live / filled / speed ⚡)
- `blue` + `cyan` → dark violet (`#a992ff` info, `#bd83ff` heading,
  `#7c3aed` ring) — the purple sheen
- `red` / `amber` → default (error / degraded keep their meaning)

To change the theme, edit that one `colors` block — do NOT hand-swap
classes across the file.

| Role | Class | Use |
|---|---|---|
| base text | `text-slate-300/400` | body |
| muted | `text-slate-500/600` | labels, captions, secondary |
| success / live | `text-emerald-400`, ring `border-emerald-500` | ok, filled, running |
| info / hint / link | `text-blue-400`, ring `border-blue-500` | hints, links, neutral status |
| error / reject | `text-red-400`, ring `border-red-500` | failures, rejects, down |
| warning / degraded | `text-amber-400`, ring `border-amber-500` | stale, paused, "no live book" |
| accent / heading | `text-cyan-400` | section headers, the ⚡ speed motif |
| panel bg | `bg-slate-800/50` | cards, callouts, hint bars |

Rules:
- Colour = meaning, never decoration. A green thing is *live/filled*, a
  red thing *failed/down*, amber *degraded* — nothing is coloured "to
  look nice."
- One accent per block. Don't stack a coloured bg + coloured border +
  coloured text; pick the ring.

## Components

- **Panel / card:** `bg-slate-800/50 rounded`, content padded; a
  surrounding ring only when the block is a typed callout
  (status/hint/error).
- **Hint bar:** `border rounded-[3px]` ring, ≤2 sentences,
  flow/direction only ("orders enter here — next → Risk"), global
  `localStorage.rsxHints` toggle. NEVER metrics or component
  re-explanations in a hint.
- **Status cell:** the word carries a semantic text colour
  (`filled`=emerald, `resting`=blue, `rejected`=red, `hung`=amber);
  a stale/degraded surface gets a ring + `opacity-40 grayscale`.
- **Tables:** slate borders, muted header row (`text-slate-500`),
  numbers right-aligned; no zebra, no heavy gridlines.

## Copy / density (inherits root CLAUDE.md)

- Lowercase info, Capitalize errors ("checking…" vs "Failed: …").
- Terse. No walls of text — a callout is ≤2 sentences; deeper detail
  lives on the component page / docs, not in a callout.
- HTMX partials: one concern per `/x/...` endpoint; render server-side.

## When you touch the UI

- New callout → surrounding ring, semantic colour, `bg-slate-800/50`.
  No banners, no left-only bars.
- New colour → only if it maps to a new *meaning*; otherwise reuse the
  table above.
- Keep the Playwright gate green (nav order, tab count, redirects).
