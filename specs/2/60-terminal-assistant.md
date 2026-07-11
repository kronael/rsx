---
status: draft
---

# 60 — Terminal Assistant (agent runner)

Status: **draft** (design captured, not implemented). The terminal's
assistant: a Claude Code agent the trader talks to from every screen, that
can see — and, within hard guards, control — the whole terminal. Design
source: `.ship/45-ARIZUKO-LLM/DESIGN-context-layering.md` (the approved
lean local-runner architecture, superseding the full-platform framing of
that sprint's PLAN.md for the near term).

Companion specs: `55-terminal.md` (the terminal UX this agent lives in —
its LLM screen, keymap, honesty rules), `54-tui-access.md` (how a trader
gets a session), `49-webproto.md` (the wire the terminal itself speaks).
The agent sits entirely client-side: it consumes the terminal's already
folded state, never the exchange wire directly.

## Table of Contents

- [Purpose](#purpose)
- [Architecture — the lean local runner](#architecture--the-lean-local-runner)
  - [Reused vs dropped](#reused-vs-dropped)
  - [Entry point: import `container.Run`](#entry-point-import-containerrun)
  - [The reply wire: `submit_turn` on the gated socket](#the-reply-wire-submit_turn-on-the-gated-socket)
  - [Backend seam and turn anatomy](#backend-seam-and-turn-anatomy)
  - [Runner config contract](#runner-config-contract)
- [Channel 1 — per-message vantage context](#channel-1--per-message-vantage-context)
- [Channel 2 — MCP: fetch and control everything](#channel-2--mcp-fetch-and-control-everything)
  - [Serving and wiring](#serving-and-wiring)
  - [Concurrency bridge](#concurrency-bridge)
  - [Read surface](#read-surface)
  - [Control surface](#control-surface)
- [Channel 3 — the agent folder (standing setup)](#channel-3--the-agent-folder-standing-setup)
- [UX — chat key and key legend](#ux--chat-key-and-key-legend)
- [Boundary](#boundary)
- [A worked turn](#a-worked-turn)
- [Build order](#build-order)
- [Backporting to arizuko](#backporting-to-arizuko)
- [Open questions for the founder](#open-questions-for-the-founder)
- [Cross-references](#cross-references)

---

## Purpose

The terminal's LLM screen today is an honest placeholder: a real context
handoff (`news.AssistantContext`, `rsx-term/news/context.go:36-54`) with no
model behind it. This spec closes that gap with an agent that is:

- **Reachable from every screen** by one keypress — a positions question
  arrives tagged from the positions screen, a depth question from the book.
- **Fully sighted.** The agent can fetch any screen's state live — book,
  trades, positions, orders, fills, news, accounting — "the model just
  knows all screens, perhaps more than the user sees."
- **Carefully handed control.** It navigates screens and drives the ladder
  cursor by sending the terminal semantic control messages — typed
  messages folded through the same guarded state machine as keystrokes,
  never synthesized keystrokes. Order entry stops at a confirm-gated
  ticket the trader must key-confirm. The agent never executes.
- **A real agent, not a chat API.** Each turn is a Claude Code run in an
  isolated Docker container with its own persistent home folder (memory,
  diary, skills), multi-turn via Claude Code's session resume.

The keeper invariants of `rsx-term/CLAUDE.md` bind the agent as they bind
every other surface: keyboard-only, never fabricate a number, offline by
default, fat-finger caps are hard blocks.

## Architecture — the lean local runner

rsx-term reuses **exactly one arizuko piece — the Claude Code runner —
unchanged.** Zero edits to arizuko source. The runner provides per-turn
Claude Code in an isolated container: agent folder mounted as `$HOME`, MCP
socket bridged in, credentials merged from the calling-process env. Prompt,
resume-session id, and turn id go in through `Input`; the reply and the
next session id come back **over the caller-owned socket** (`submit_turn`),
not the return value — see [the reply wire](#the-reply-wire-submit_turn-on-the-gated-socket).
Everything RSX-specific lives in rsx-term and is fed through contracts the
runner already honors.

### Reused vs dropped

Reused (all verified in arizuko source, sibling checkout):

| Reused | Where | What it gives |
|---|---|---|
| `container.Run(cfg, folders, in Input) Output` | `arizuko/container/runner.go:149` | one blocking call = one isolated turn; the return is the end-of-turn signal, not the reply |
| `Input{Prompt, SessionID, MessageID, Folder, Topic, ExternalMCP, Model, QueryTimeoutMs, ...}` | `runner.go:75-116` | prompt + resume id + turn id in; `SessionID` non-empty = resume; `MessageID` becomes the `turn_id` replies carry |
| `Output{Status, ExitCode, MessageCount}` | `runner.go:118-129, 460` | end-of-turn/error signal ONLY — the struct's `Result`/`NewSessionID` fields are never assigned by `Run`; the reply rides `submit_turn` |
| `ExternalMCP: true` | `runner.go:104-106, 230` | "the MCP socket is owned by the caller" — the seam that lets rsx-term serve its own MCP **and receive the reply**; already upstream |
| `submit_turn` / `submit_status` wire | agent side `ant/src/index.ts:90-108, 151-163`, `ant/src/mcp.ts:44-90`; host shape `ipc/ipc.go:409-563` | reply + session id + mid-turn status delivered to whatever listens on the gated socket |
| MCP bridge plumbing | ipc dir bind-mount `runner.go:549-556`; ant synthesizes `mcpServers.arizuko` = socat → `/run/ipc/gated.sock`, eager (`ant/src/mcp-servers.ts:54-59`) | in-container Claude Code dials whatever listens on the host socket, eagerly every turn |
| Folder-as-`$HOME` mount + seeding | `runner.go:519-522` (mount), `seedSettings` `runner.go:764-856` | the agent's whole world is a local directory rsx-term authors |
| Isolation + lifecycle | `runner.go:270-298, 329` | spawn / idle-timeout / hard timeout / graceful-stop / kill |
| Credentials | `readSecrets()` merges the calling-process env (`runner.go:261, 707-720`) | `ANTHROPIC_API_KEY` in rsx-term's env flows to the container; no secrets broker |

Dropped — the entire arizuko platform plane, each replaced by something
rsx-term already has:

| Dropped | Replaced by |
|---|---|
| routd prompt assembly | rsx-term renders the prompt string itself |
| routd `sessions` table | rsx-term keeps `map[thread]sessionID` fed by `submit_turn.session_id` |
| webd `/chat/{token}` | a direct function call in; the caller-owned socket back |
| grants / ACL filtering | moot — rsx-term's own MCP server exposes only what it chooses |
| authd / tokens / rate limits / proxyd / onbod / runed / routing / social | single local user, no network surface |
| crackbox egress proxy | **nothing — this is open question 0.** Dropping the plane drops the platform's egress mitigation too |

Conversation context (multi-turn memory) is Claude Code's own session
resume plus the agent folder's MEMORY.md/.diary — mechanisms rsx-term never
interprets. Market context (vantage blocks, MCP payloads) is
rsx-term-authored text and JSON the runner never interprets. The two stay
separate by construction.

### Entry point: import `container.Run`

Three ways exist to invoke the unchanged runner; the library import wins:

| Option | Verdict |
|---|---|
| **(a) Go library: import `container.Run`** via a `go.mod` `replace` to the local arizuko checkout | **Chosen.** A public function called with public types — genuinely unchanged. All host-side machinery (mount construction, settings + annotation seeding, `Input` marshaling to container stdin, stderr activity accounting, stop/kill lifecycle) executes exactly as upstream wrote it. Zero drift risk. |
| (b) `docker run` the `arizuko-ant` image directly | Rejected. The image's stdin-JSON entrypoint is only half the runner; the seeding, mounts, marshaling, and lifecycle live in the Go package **host-side**. Reimplementing them in rsx-term is a fork by copy — it drifts on every arizuko release. |
| (c) Run `runed`, POST `/v1/runs` | Rejected. Drags the queue/broker/DB/authz plane — exactly the fat messaging plane the pivot cut — and its `RunRequest` expects routd-rendered prompts and grant sets that don't exist here. |

One Go dependency, no daemon, no image-contract reimplementation. The
import sits behind the same env opt-in as every other dial site (the
offline default stays byte-identical: unset env → nil backend → zero
constructions, zero dials). It is NOT free at runtime, though — the real
prerequisites (docker, the built image, a live arizuko checkout, a Go
directive bump) are pinned in [the config contract](#runner-config-contract).

### The reply wire: `submit_turn` on the gated socket

The runner never returns the reply. `Run` discards container stdout
(`cmd.Stdout = io.Discard`, `runner.go:219`) and its success value is
`Output{Status: "success", ExitCode, MessageCount}` (`runner.go:460`) —
the struct's `Result`/`NewSessionID` fields exist but are never assigned
in this path. The agent delivers its reply **out-of-band**, over the same
unix socket the MCP tools ride:

- At end of turn, ant's `deliverTurn` (`ant/src/index.ts:90-108`) fires a
  newline-framed JSON-RPC request, method `submit_turn`, at
  `/run/ipc/gated.sock` (`ant/src/mcp.ts:3, 44-90`), carrying
  `{turn_id, session_id, status, result?, error?, timed_out?}`. The
  `session_id` is harvested from the SDK's `system_init` event
  (`index.ts:323-325`) — this is the resume id, nothing else carries it.
- Mid-turn `<status>` blocks flow the same way as
  `submit_status{turn_id, text}` (`index.ts:151-163`, `mcp.ts:88-90`).

With `ExternalMCP: true`, rsx-term owns that socket — so rsx-term's socket
handler MUST speak arizuko's ipc wire protocol, not bare mcp-go:

- **Framing.** One JSON-RPC object per `\n`-terminated line; responses
  likewise (`ipc/ipc.go:421-430`; `mcp.ts:44-82` is the client side).
- **Demux before MCP.** Parse each line's `method`: `submit_turn` → the
  turn handler, `submit_status` → the status handler, everything else →
  mcp-go's `srv.HandleMessage` (mirror `serveConn`, `ipc/ipc.go:409-478`).
  A stock mcp-go server alone answers `submit_turn` with
  method-not-found, ant logs the failure and drops the reply on the floor
  — the demux is not optional.
- **Handlers.** Validate `turn_id`, deliver, ack `{"result":{"ok":true}}`
  (mirror `handleSubmitTurn` / `handleSubmitStatus`,
  `ipc/ipc.go:480-563`). `ipc.TurnResult` (`ipc/ipc.go:120-135`) is the
  payload shape.
- **Socket hygiene.** Create, `chmod 0660`, chown to the container uid,
  and optionally SO_PEERCRED-verify peers (`ipc/ipc.go:350-358`; uid
  selection as in `runner.go:232-238`). Upstream also bounds accept
  fan-out; with one container that is taste, not load-bearing.
- **Correlation.** `turn_id` echoes `Input.MessageID`; left unset, ant
  mints `boot-<ts>` (`index.ts:430`) and the reply cannot be matched to a
  thread. Set `MessageID` to a fresh turn id on every `Ask`. The `:N`
  turn-id suffixes only appear for in-container follow-up messages via the
  ipc input dir, which rsx-term doesn't use — exact match suffices.

The socket is a bind mount (`runner.go:549-556`), so this contract
survives any container network mode — including the locked-down egress of
open question 0.

### Backend seam and turn anatomy

The existing assistant seam (`rsx-term/assistant/client.go:61-93` —
`Ask(topic, content)` → `Reply/Status/Failed` events,
`client.go:34-52`) generalizes to an interface with two implementations
emitting the same three events:

- `Client` (exists) — HTTP `/chat/{token}` to a full arizuko deployment;
  the LATER layer.
- `Runner` (this spec) — local `container.Run` + the gated-socket reply
  wire; the near-term target.

Selection: `RSX_TERM_ASSIST=local:<agent-dir>` → Runner;
`RSX_TERM_ASSIST=http(s)://…` → Client; unset → nil backend, the
byte-identical offline placeholder (keeper invariant).

A `Runner.Ask` turn:

1. **Gate:** one turn in flight per thread — same rule the HTTP path
   enforces (`assistBusy` gate, commit 4bec07c). A second ask on a busy
   thread is refused honestly; the draft survives in the input.
2. Ensure the gated-socket server is up (once per process) at
   `groupfolder.IpcSocket(ipcDir)` — listening BEFORE the spawn, speaking
   the reply wire above.
3. Mint a turn id, record `turnID → thread`, and build
   `container.Input{Folder: "rsx/term/desk", GroupPath: <agent-dir>,
   Topic: thread, SessionID: sessions[thread], MessageID: turnID,
   Prompt: vantage + "\n---\n" + text, ExternalMCP: true,
   Model: <configured>, QueryTimeoutMs: <configured>}` with the config
   contract below.
4. `container.Run(...)` blocks in one named goroutine for the container's
   lifetime. Its return is the end-of-turn/error signal ONLY.
5. The reply arrives on the socket: `submit_turn` → correlate `turn_id` →
   store `session_id` in `sessions[thread]` → emit `Reply` (or `Failed` on
   error status) — never fabricated content. Mid-turn `submit_status` →
   emit `Status` (the pane's existing busy cue, `client.go:42-45`) — no
   arizuko change needed for streaming progress.
6. Reconcile: if `Run` returns with no `submit_turn` seen for the turn,
   emit `Failed` from `Output.Error` — the honest fallback. Clear the
   thread's busy gate either way.

A "thread" near-term is one terminal-session-scoped id per chat entry;
persisting the `thread→sessionID` map across restarts is a follow-up, not
part of the first cut.

### Runner config contract

`container.Run(cfg *core.Config, folders *groupfolder.Resolver, in Input)`
reads more of `cfg` than "minimal" suggests, and several zero values have
sharp failure modes. rsx-term builds this once at backend construction:

| Field | Value | Why (and what the zero value does) |
|---|---|---|
| `Name` | `rsx` | container name prefix + `ARIZUKO_ASSISTANT_NAME` |
| `Image` | `arizuko-ant` | must exist locally (built from the arizuko checkout); ships node + socat |
| `Timeout` | ~5m | hard total-turn kill (`runner.go:329`). arizuko's own env default is 60m (`core/config.go:167`) — an hour of "busy" on a wedged turn; a hand-built zero disables the watchdog |
| `IdleTimeout` | ~2m | stderr-silence stop, reset per `[ant]` line. **Zero = `time.AfterFunc(0)` stops the container instantly** (`runner.go:295-298`) — it MUST be set |
| `GroupsDir` | `<state-root>/groups` | world share mount root (`runner.go:536-543`); empty litters relative dirs in the CWD |
| `IpcDir` | `<state-root>/ipc` | socket dir, resolved via `folders` (`groupfolder.IpcSocket`, `groupfolder/folder.go:152`) |
| `WebDir` | `<state-root>/web` | tier≤2 web mounts (`runner.go:598-627`); moot at tier 3 but set it against drift |
| `HostAppDir` | absolute path to a real arizuko checkout | mounted RO at `/opt/arizuko` **unconditionally** (`runner.go:526-530`) — empty yields an invalid `-v` spec and every spawn fails. Also the host-side source for the output-style/migrate seeds |
| `Timezone` | `UTC` | container `TZ` |

`folders` is `&groupfolder.Resolver{GroupsDir, IpcDir}` over the same
values.

**Folder: `rsx/term/desk`, not `rsx`.** A `Folder` with no `/` is
root/tier-0 (`runner.go:150`), which drags the platform's operator plane
into the desk agent: the whole groups tree mounts at `/var/lib/groups`
(`runner.go:629-634`), the public web tree plus writable
`~/public_html`/`~/private_html` slots mount — creating `pub/rsx` and
`priv/rsx` dirs under `WebDir` (`runner.go:611-627`) — and the agent runs
with `ARIZUKO_IS_ROOT=1` (`runner.go:796`). A tier-3 folder
(`rsx/term/desk`; tier = slash-count + 1, `runner.go:65-73`) sheds the web
and groups mounts entirely. What remains: the agent dir as `$HOME`
(`GroupPath` overrides the resolver), the world share at
`<GroupsDir>/rsx/share`, the ipc mount, and `/opt/arizuko` RO.

**Model and timeouts are deliberate, not defaulted.** An empty
`Input.Model` falls to ant's hardcoded default (`claude-opus-4-8`,
`ant/src/backend/claude.ts:29, 280`) — expose it
(`RSX_TERM_ASSIST_MODEL`) and pass it through. An unset
`Input.QueryTimeoutMs` falls to ant's 15-minute hardcode
(`claude.ts:23`). The Runner backend has no HTTP-client-style idle cutoff
— `Run` returning is its only end signal — so set `QueryTimeoutMs` just
under `cfg.Timeout`: the wedged turn then ends as ant's graceful timeout
summary (a real `submit_turn`) instead of a silent kill.

**Runtime prerequisites** — the import is one `go.mod` line, but it is not
"compile-time only": the docker CLI on PATH; the `arizuko-ant` image
built; a real arizuko checkout at `HostAppDir`; and rsx-term's `go`
directive raised to ≥ arizuko's (`go 1.25.5`; rsx-term is `go 1.24.0`
today) or the module import won't resolve. All behind the env opt-in —
unset `RSX_TERM_ASSIST` constructs nothing and dials nothing.

## Channel 1 — per-message vantage context

Every message carries **the user's vantage**: what screen they were on,
what they had focused, and the terminal's state at that moment. Per the
founder: rich and full — send the full state tagged with the vantage, so
later view-tailored trimming is a provider change, not a protocol change.

Block shape (generalizes today's `Render`,
`rsx-term/assistant/prompt.go:59-84`, which handles only news/freeze
handoffs):

```
[RSX VANTAGE]
screen: book            # book|news|positions|accounting|...
focus: PENGU ladder, cursor 41250 / frozen row "~10s window"
[RSX STATE]             # rich/full for now, same honesty rules
market: rsx · PENGU  mid 41258.5  at 2026-07-11T14:32:07Z
asks: ...  bids: ...    # top-N per side
position: +8  entry 41180  uPnL +12.40
open orders: bid 41200×5
fills this session: 3
news: <selected headline, if any>
```

- **Owner:** `rsx-term/assistant/vantage.go` — a pure
  `RenderVantage(v Vantage) string`, unit-tested like `prompt.go`. The UI
  fills `Vantage` from what it already folds (the assist snapshot, fills
  via `position.ApplyFill`) plus the current screen/focus. The existing
  `news.AssistantContext` becomes one input (the news/freeze vantage)
  rather than the only shape.
- **Injection:** prepended to `Input.Prompt` every turn. The runner has no
  other per-turn context channel (the platform's `<autocalls>` block is
  routd-side, and routd is dropped) — content-prepend IS the channel.
  Claude Code's session carries history, so the agent sees the vantage
  evolve across turns naturally.
- **The runner decorates first.** `prepareInput` (`runner.go:463-510`)
  prepends its own platform annotations ahead of our prompt on every
  spawn: `[resolve] Invoke /resolve now…` unconditionally
  (`runner.go:502-504`), a `Topic session:` note when `Topic` is set, the
  folder's recent episodes/diary/`work.md`, and `[pending migration]…`
  whenever the folder's skills version trails the checkout
  (`runner.go:464-473`). We absorb rather than fight: the agent-dir
  CLAUDE.md defines what resolving means for a desk agent, the template
  seeds `.claude/skills/self/MIGRATION_VERSION` to silence the migration
  nag, and the diary/episodes prepends are the agent's own accumulated
  memory — a feature. See open question 3.
- **Honesty + bounds:** the unchanged keeper rules — dash unknowns, label
  frozen far rows as aggregate windows (`~`), timestamp every block, never
  fabricate (`rsx-term/notes/honesty.md`). "Rich and full" still means
  bounded: top-N levels per side (~5), one screen's focus detail, on the
  order of 30 lines. The vantage is *what the user sees*; it is
  deliberately NOT the agent's data ceiling — that is MCP's job.

## Channel 2 — MCP: fetch and control everything

rsx-term itself is an MCP server; the containerized agent reaches it
through mechanisms the runner already ships — no new arizuko code path.

### Serving and wiring

**In-process, not sidecar.** The state lives in the bubbletea model; a
sidecar would need its own feed and re-fold everything. rsx-term serves a
unix socket using the same library arizuko uses
(`github.com/mark3labs/mcp-go`) from a named goroutine in
`rsx-term/assistant/mcpserver.go`, behind the same env opt-in.

**One socket, two protocols.** The listener at
`groupfolder.IpcSocket(ipcDir)` is the SAME one that receives
`submit_turn`/`submit_status` — the demux in
[the reply wire](#the-reply-wire-submit_turn-on-the-gated-socket) hands
every non-submit line to mcp-go's `HandleMessage`. The MCP tool surface
is that default branch.

Primary wiring: `Input.ExternalMCP: true` + rsx-term listening before the
spawn. In-container Claude Code connects via a `mcpServers.arizuko` socat
entry that **ant synthesizes itself with `alwaysLoad: true`**
(`ant/src/mcp-servers.ts:54-59`); any stale settings.json copy is dropped
first (`mcp-servers.ts:26`) — including the runner's own seeded one
(`runner.go:840-844`), so the eager-load decision is ant's, same socket
either way. Eager means the full `rsx_*` tool surface is omnipresent,
which is what "the model just knows all screens" needs. Accepted nit: the
server alias reads "arizuko" in-container; tool names (`rsx_*`) stay ours.

Fallback (kept in the pocket, only if the primary misbehaves): a second
socket inside the mounted folder plus an operator `mcpServers` entry —
`seedSettings` round-trips `.claude/settings.json` and preserves foreign
entries (`runner.go:770-777, 836-844`), and ant loads them deferred
(`loadAgentMcpServers`, `mcp-servers.ts:21-31`). Costs: tools become
discoverable (tool-search) rather than omnipresent, and it covers tools
ONLY — the reply wire still runs over the gated socket regardless.

### Concurrency bridge

MCP handlers run on socket goroutines; the model folds on the tea update
loop. Two paths, no shared mutable state:

- **Read tools → `StateMirror`.** After each fold the update loop
  publishes an immutable snapshot (atomic pointer swap) of
  book/positions/orders/fills/news/screen. Handlers read the pointer — no
  locks on the fold path, always self-consistent.
- **Control tools → `p.Send(msg)` with semantic messages ONLY.** Each
  control verb maps to a dedicated typed message (`assistGotoMsg`,
  `assistCursorMsg`, …) that the update loop folds through the same
  screen state machine that guards keystrokes — same checks, same
  refusals. Injecting `tea.KeyMsg` is FORBIDDEN: synthesized keystrokes
  would hand the agent every key on the board, including the
  confirm/place key, and defeat the confirm gate. The confirm/submit
  transition must have NO injectable message equivalent — its only
  producer is the real key handler, so the fat-finger cap stays a hard
  block in a path only the trader's fingers reach (`fire()`,
  `rsx-term/ui/update.go:546-566`). The handler waits on a reply channel
  with a short deadline; a timeout returns an honest error.

### Read surface

Per screen, JSON out, all served from the terminal's own folded state —
which makes positions/fills honest by construction: they come from the
same `ApplyFill` folds the trader sees, the single honest source.

| Tool | Returns |
|---|---|
| `rsx_get_screen` | current screen, focus, cursor — the live vantage |
| `rsx_get_orderbook(symbol?, depth≤10)` | live bids/asks px×qty, mid, ts |
| `rsx_get_trades(symbol?, limit≤50)` | recent prints |
| `rsx_get_positions()` | net/entry/uPnL per symbol (client-folded) |
| `rsx_get_orders()` | resting orders |
| `rsx_get_fills(limit)` | session fill log |
| `rsx_get_news(limit)` | headlines + selected marker |
| `rsx_get_accounting()` | whatever the accounting screen folds |

### Control surface

Start narrow, widen deliberately:

| Tool | Action |
|---|---|
| `rsx_goto_screen(screen)` | navigate: book/news/positions/accounting/chat |
| `rsx_focus_symbol(symbol)` | switch the active market |
| `rsx_set_cursor(px)` / `rsx_freeze_row(px)` | drive the ladder cursor / book-microscope freeze |
| `rsx_propose_order(side, px, qty)` | **confirm-gated**: renders a pre-filled order ticket; the TRADER presses the key to send. Returns `"pending trader confirm"`, never a fill. |

However far this table widens, the bridge rule holds: every verb is a
semantic message; none may synthesize keys. Order entry never executes
agent-side — the fat-finger cap and the keyboard confirm stay in the
trader-only submit path (keeper: hard block, not warning). Whether
`rsx_propose_order` exists at all is founder question 1.

**Push/pull dedup rule:** vantage = what the user is looking at, frozen
and labeled, one per message; MCP = anything, live, agent-initiated. The
vantage block carries no promise of completeness — the agent folder's
CLAUDE.md tells the agent "for anything current or off-screen, call the
tools".

## Channel 3 — the agent folder (standing setup)

The general RSX know-how — how the agent *thinks* — is not per-message; it
is the agent folder the runner mounts as `$HOME` every spawn:

```
<agent-dir>/                      # RSX_TERM_ASSIST=local:<agent-dir>
  CLAUDE.md                       # THE standing instructions
  PERSONA.md                      # voice/register; CLAUDE.md points here
  .claude/skills/rsx-*/SKILL.md   # optional deep topics, on-demand
  .claude/skills/self/MIGRATION_VERSION  # seeded to silence the migration nag
  MEMORY.md, .diary/, work.md     # the agent's own accumulation — untouched
```

The CLAUDE.md is the agent's operating manual: identity + honesty rules
("not in the snapshot → say so; dash unknowns; a frozen far row is an
aggregate window"); the `[RSX VANTAGE]`/`[RSX STATE]` block contract; the
tool map (which `rsx_*` tool answers which question; vantage for "what the
user sees", tools for "now / off-screen"); what the runner's `[resolve]`
annotation means for a desk agent (classify the question, recall memory,
match an `rsx-*` skill — then answer); fixed-point conversion (tick/lot,
raw i64); reading the ladder (live ~100ms bins vs time-weighted windows);
position folding + the uPnL formula; funding mechanics (zero-sum per
symbol); control etiquette (navigate freely, never propose an order
unasked). Deep single topics graduate to `.claude/skills/rsx-*/SKILL.md`
(pure markdown, on-demand) only when the CLAUDE.md outgrows a few hundred
lines — start with one file.

**PERSONA.md is a file, not a wire field.** The runner reads it into
`Input.Persona` (`runner.go:491-499`), but that field is dead on the wire:
Go marshals it as `persona` and ant only reads `soul`
(`ant/src/index.ts:22, 48`) — it never reaches the system prompt. The
voice works the only way that actually functions: CLAUDE.md instructs the
agent to read PERSONA.md and adopt it.

**Authoring/ownership:** source of truth in this repo —
`rsx-term/assistant/agentdir/` (CLAUDE.md, PERSONA.md, skills, the
MIGRATION_VERSION seed) — installed/synced by
`make assist-agent-dir AGENT_DIR=...`. rsx-term writes it; the runner
mounts it; arizuko never needs to understand a word.

**What each spawn writes into the folder** (platform-owned, tolerated):
`.claude/output-styles/` and the `migrate` skill are force-refreshed from
the checkout (`runner.go:744-762`), and `.claude/settings.json` is
rewritten — foreign keys preserved, `env`/`permissions`/`mcpServers`
asserted (`seedSettings`, `runner.go:764-856`). The full platform skill
copy (`seedSkills`, 90+ dirs) does NOT run here — it is reachable only via
`SetupGroup`/`seedGroupDir` (`runner.go:940-978`), which rsx-term never
calls, so the folder keeps only the skills we author.

## UX — chat key and key legend

- **Chat opens from every screen, by keypress, never mouse** (keyboard-only
  keeper invariant). One new `binding` row (`actOpenChat`) in the existing
  keymap table (`rsx-term/ui/keymap.go:58-64`) — the table already drives
  dispatch, the per-screen hint line, and the `?` help overlay from one
  source of truth, so the chat key lands in hints/help/rebinds with zero
  new mechanism. Pressing it switches to the chat pane and attaches the
  vantage of the screen the user was on.
- **Key legend, every screen, top-right.** Each screen renders its
  available keys as a compact corner legend generated from the same keymap
  `hint` fields — a render change in `view.go`, not a new UI system — so
  the chat key and each screen's verbs are always visible.

## Boundary

- **rsx-term owns domain data + the agent's world.** It structures the
  per-message vantage, serves the MCP surface (state + control of every
  screen), hosts the socket the reply comes back on, and authors the agent
  folder's standing setup. It alone knows what a book, fill, mid,
  position, or funding rate means.
- **arizuko's runner just executes Claude Code in isolation.** A
  directory, a prompt string, a socket in; an end-of-turn signal out — the
  reply itself arrives back over the caller's socket. It never parses
  market data.

## A worked turn

**T+0, from the book.** Trader freezes a PENGU row, hits the chat key,
types "is this wall real?". rsx-term renders the book-vantage block
(screen: book, frozen row labeled `~10s window`, full state) + text →
`Runner.Ask` → `container.Run` spawns Claude Code on the agent folder; its
CLAUDE.md primes it; it answers from the vantage. The reply lands as
`submit_turn` on the gated socket; its `session_id` is stored for the
thread. *(Channels 1+3.)*

**T+2min, follow-up.** "what about now?" — new turn, same thread
(`SessionID` resumes; history is Claude Code's). The fresh vantage shows
the book has moved; the agent compares, and calls
`rsx_get_orderbook(depth=10)` for depth below the vantage's top-5 — served
from the StateMirror, live. *(1+2+3.)*

**Same turn, agent acts.** "show me" → the agent calls
`rsx_goto_screen("book")` + `rsx_set_cursor(41250)`; the tea loop folds
the semantic messages through the same guarded state machine as
keystrokes; the trader watches the cursor land. Any order would stop at a
confirm ticket. *(2, control.)*

## Build order

1. **A — runner backend + the reply wire.** Backend interface extraction +
   `assistant/runner.go` (`container.Run` in a named goroutine, per-turn
   `MessageID`, `thread→sessionID` map, one-turn-per-thread gate, env
   selection, the config contract) + the gated-socket server in
   `assistant/mcpserver.go`: newline-framed JSON-RPC demux —
   `submit_turn`/`submit_status` handlers (turn correlation → `Reply` /
   `Status` / `Failed`, ack `{"ok":true}`), everything else to a
   tool-less mcp-go `HandleMessage` — plus socket perms/peer-uid. Also:
   the agent-dir template (CLAUDE.md absorbing `[resolve]`,
   MIGRATION_VERSION seed) + `make assist-agent-dir` + the rsx-term
   `go.mod` directive bump (≥1.25.5). An "empty mcp-go server on the
   socket" is NOT sufficient — without the demux, `submit_turn` is
   method-not-found and no reply ever arrives. Done-check: multi-turn chat
   from the pane, replies and mid-turn statuses arriving via
   `submit_turn`/`submit_status` correlated by `MessageID`, fully local,
   offline goldens untouched.
2. **B — vantage provider + chat key.** `vantage.go` with per-screen focus
   tagging; the `actOpenChat` binding row; the keymap-generated top-right
   key legend on every screen. Depends on A only for end-to-end testing.
3. **C — MCP read tools.** StateMirror + the eight read tools, registered
   on step A's mcp-go server. Independent of B; after A.
4. **D — MCP control verbs.** Navigation set as semantic messages via
   `p.Send` (never `tea.KeyMsg`); then (founder gate) `rsx_propose_order`
   with the confirm ticket.
5. **E — later layer.** Full arizuko platform per the sprint PLAN.md; swap
   the backend to the existing HTTP `Client` behind the same interface.

## Backporting to arizuko

Near-term is **zero arizuko changes** — everything rides public,
already-shipped contracts: `container.Run`, `Input.ExternalMCP`,
`Input.MessageID`, and the `submit_turn`/`submit_status` socket wire.
(Streaming status needs no backport at all — `submit_status` already
reaches the caller's socket; wiring it to the pane's existing `Status`
event is build step A.) Strictly later, optional upstream PRs so the
embedding stays a contribution instead of a divergence:

- **B1 — pin the embedder contract.** Document "library mode"
  (`container.Run` + `Input.ExternalMCP` + `Input.MessageID` + the
  `submit_turn`/`submit_status` wire protocol and `ipc.TurnResult` shape)
  in arizuko's EXTENDING.md and mark those stable, so routd/runed/ant
  refactors don't silently break embedders.
- **B2 — lean embedder spawn.** A cfg flag to skip the platform seeds
  (output-styles, migrate skill) and `prepareInput`'s prompt annotations
  for non-platform folders. (Not skill seeding — `seedSkills` runs only in
  `SetupGroup`, which embedders never call.)
- **B3 — the vantage/tool-map prompt patterns** proven locally become the
  PERSONA/skill content of the full-platform folder when the LATER layer
  (threads/social per the sprint PLAN.md) arrives.

## Open questions for the founder

0. **Egress — the agent container gets OPEN INTERNET (safety).** With a
   zero `Input.Egress`, `registerEgress` is a no-op
   (`container/egress.go:47-49, 90-93`) and `--network` is never passed
   (`runner.go:660-674`) — the container lands on Docker's default bridge
   with unrestricted egress. The platform's mitigation (the crackbox
   allowlist proxy) lives in the plane we drop. The threat is concrete:
   news headlines are attacker-controlled text in the agent's context; a
   prompt-injected agent can read positions/orders/fills via `rsx_*` and
   exfiltrate them anywhere — it must reach `api.anthropic.com`, so it can
   reach the internet. Options: (a) accept and document — single local
   user on a trusted box, but the injection source is untrusted by
   definition; (b) put the container on a restricted network with an
   allowlisting proxy (arizuko's own crackbox via `Input.Egress`, or a
   local equivalent) so egress is Anthropic-only. The reply/MCP socket is
   a bind mount and works under either. If (b), note tier-0/1 folders get
   an auto-appended `*` allowlist (`runner.go:171-182`) — one more reason
   the desk agent is tier-3. Recommendation: (b) before the first session
   against a real account; (a) is defensible only with mock/paper venues.
1. **Control ceiling.** Does `rsx_propose_order` (confirm-gated ticket,
   trader presses the key) belong in the first control set, or is control
   strictly navigation/focus until further notice? Recommendation: ship
   navigation first, add the confirm-gated ticket second — never an
   unattended execute verb, and never an injectable confirm.
2. **Agent folder lifecycle.** One per-machine folder
   (`local:~/.rsx/agent`, memory/diary accumulate across sessions and
   desks) or per-desk folders? Recommendation: singleton — the accumulated
   MEMORY.md is half the value; revisit if multi-desk arrives.
3. **Absorbing the runner's platform annotations.** `prepareInput`
   decorates every prompt (`runner.go:463-510`): `[resolve]` always,
   `[pending migration]` until the folder's skills version matches, plus
   topic/episodes/diary prepends. Near-term we absorb (CLAUDE.md defines
   `/resolve` for a desk agent; the template seeds MIGRATION_VERSION; the
   diary/episodes prepends are the agent's own memory — useful). Is
   absorb-and-tolerate right, or is the lean-spawn flag (backport B2)
   worth being the first upstream PR? Recommendation: absorb now; PR only
   if the annotations measurably pollute replies.

## Cross-references

| Concern | Where |
|---------|-------|
| Terminal UX, LLM screen, keymap, honesty | 55-terminal.md |
| Terminal access (SSH / web session) | 54-tui-access.md |
| Client wire protocol (what the terminal folds) | 49-webproto.md |
| Keeper invariants (keyboard-only, never fabricate, offline default) | `rsx-term/CLAUDE.md` |
| Design source + arizuko line cites | `.ship/45-ARIZUKO-LLM/DESIGN-context-layering.md` |
| Reply wire host-side reference | `arizuko/ipc/ipc.go:409-563`; agent-side `arizuko/ant/src/index.ts`, `ant/src/mcp.ts` |
| Existing handoff + prompt renderer | `rsx-term/news/context.go`, `rsx-term/assistant/prompt.go` |
