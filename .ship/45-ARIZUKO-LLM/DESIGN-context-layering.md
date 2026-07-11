# DESIGN — Context layering: the terminal feeds the agent (local-runner architecture)

Sprint `.ship/45-ARIZUKO-LLM`. Supersedes the full-platform framing of
PLAN.md Phases 0-1 for the near term: the founder's pivot is **reuse ONLY
arizuko's Claude Code runner** — container isolation, folder mount, MCP
bridge — driven directly by rsx-term against a local agent folder. Hard
constraint: **the runner runs UNCHANGED — zero edits to arizuko source,
near-term.** Every RSX specific lives outside arizuko and is fed through
contracts the runner already honors. The full arizuko platform (threads
across channels, social, multi-tenant) is a LATER layer. Design only — no
production code here.

## 0. Boundary and pivot

- **rsx-term owns DOMAIN DATA + the agent's world.** It structures the
  per-message vantage context, serves the MCP surface (fetch + control of
  every screen), and authors the agent folder's standing setup
  (CLAUDE.md/PERSONA.md/skills). It alone knows what a book, fill, mid,
  position, or funding rate means.
- **arizuko contributes exactly one thing near-term: the RUNNER.** Per-turn
  Claude Code in an isolated Docker container, group folder mounted as
  `$HOME`, MCP socket bridged in, session resume in/out. It executes; it
  never parses market data. Explicitly OUT for now: routd, webd, authd,
  proxyd, onbod, runed (the queue daemon), grants/ACL, the sessions table,
  routing, social. No fat messaging plane.
- The two "contexts" stay separate by construction: *conversation* context
  (multi-turn memory) is Claude Code's own session resume + the agent
  folder's MEMORY.md/.diary — mechanisms rsx-term never interprets;
  *market* context (vantage blocks, MCP payloads) is rsx-term-authored text
  and JSON the runner never interprets.

## 1. The minimal reused arizuko surface (verified in source)

| Reused | Where | What it gives us |
|---|---|---|
| `container.Run(cfg, folders, in Input) Output` | `arizuko/container/runner.go:149` | one blocking call = one isolated turn; plain importable Go function |
| `Input{Prompt, SessionID, Folder, Topic, Persona, SystemMd, GroupPath, ExternalMCP, ...}` | `runner.go:74-107` | prompt in via JSON-over-stdin (`runner.go:263-268`); `SessionID` non-empty = resume |
| `Output{Status, Result, NewSessionID, Error, ExitCode}` | `runner.go:118-129` | the reply text + the session id to resume next turn |
| **`ExternalMCP: true`** | `runner.go:104-106, 231` | "the MCP socket is owned by the caller … skip the in-container ServeMCP, just mount the ipc dir" — the exact seam that lets rsx-term serve its own MCP; **already upstream, zero fork** |
| MCP bridge plumbing | `runner.go:838-843` (settings.json `mcpServers.arizuko` = `socat STDIO UNIX-CONNECT:/run/ipc/gated.sock`), ipc dir bind-mount `runner.go:546-553` | in-container Claude Code dials whatever listens on the host socket; eager-loaded every turn (`ant/src/mcp-servers.ts`) |
| Folder-as-`$HOME` mount + seeding | `runner.go:512-520` (mount), `prepareInput` reads PERSONA.md/SYSTEM.md (`runner.go:491-499`), `seedSettings` round-trips settings.json (`runner.go:764-855`) | the agent's whole world is a local directory we author |
| Isolation + lifecycle | spawn/idle-timeout/graceful-stop/kill (`runner.go:271-299`), egress registration (nil allowlist = unconstrained, per `runed/api/v1/types.go` comment) | container isolation kept, as required |
| `groupfolder.Resolver{GroupsDir, IpcDir}` | `groupfolder/folder.go:36-39` | plain struct, no daemon |
| `arizuko-ant` image | `cfg.Image`, built by `make images` | the runtime; untouched |
| Credentials | `readSecrets()` merges the **calling process env** (`runner.go:261`) | `ANTHROPIC_API_KEY` in rsx-term's env flows to the container; no secrets broker |

**Dropped** (and what replaces each): routd prompt assembly → rsx-term
renders the prompt string itself; routd `sessions` table → rsx-term keeps
`map[thread]sessionID` fed by `Output.NewSessionID`; webd `/chat/{token}` →
a direct function call; grants/ACL filtering (`ipc/ipc.go:906-912`) → moot,
rsx-term's own MCP server serves only what it chooses to expose; authd /
tokens / rate limits → single local user, no network surface at all.

### 1.1 Entry point — three ways to invoke the unchanged runner, one winner

| Option | What it is | Verdict |
|---|---|---|
| **(a) Go library: import `container.Run`** | rsx-term (Go) imports `github.com/kronael/arizuko/{container,core,groupfolder}` via a `replace` to the local checkout and calls `Run` with a caller-built `Input` | **RECOMMENDED.** Genuinely unchanged — a public function called with public types. All host-side seeding (`prepareInput` persona read, `seedSettings` settings round-trip, `seedSkills`, `buildMounts`, egress, idle-timeout lifecycle) executes exactly as upstream wrote it. Zero drift risk. |
| (b) Invoke the `arizuko-ant` image directly | `docker run` the image per its stdin-JSON entrypoint contract, mounts built by hand | Rejected. The entrypoint contract is only half the runner: settings/skills/persona seeding, mount construction, `Input` marshaling (`runner.go:263-268`), stderr `[ant]`-line parsing into `Output.Result`, and stop/kill lifecycle (`runner.go:271-299`) all live in the Go package, **host-side**. Reimplementing them in rsx-term is a fork by copy — it drifts on every arizuko release. Leanest-looking, most divergent in practice. |
| (c) Run `runed` as-is, POST `/v1/runs` | the platform's runner daemon (`runed/api/v1/types.go`) | Rejected. runed drags the queue/broker/DB/authz plane (`runed/broker.go`, migrations, service tokens) — exactly the "fat messaging" the founder cut. Its `RunRequest` also expects routd-rendered prompts and grant sets we don't have. |

So: one Go dependency, no daemon, no image-contract reimplementation. The
import is compile-time only and sits behind the same env opt-in as every
dial site (offline default untouched).

## 2. Turn anatomy — how rsx-term drives a turn

New backend behind the existing assistant seam (`rsx-term/assistant/`
already defines the event contract: `Ask(topic, content)` →
`Reply/Status/Failed` events, `client.go:61-93`). Extract the interface;
two implementations:

```go
type Backend interface {          // rsx-term/assistant/backend.go
    Enabled() bool
    Ask(topic, content string)    // non-blocking; one named goroutine per turn
    Events() <-chan any
}
// Client (exists today, client.go) — HTTP /chat/{token} → full arizuko, LATER layer.
// Runner (new, runner.go)        — local container.Run, NEAR-TERM target.
```

`Runner.Ask` in its named goroutine:

1. Ensure the MCP socket server is up (once per process, §4) at
   `<ipcDir>/rsx/gated.sock`.
2. Build `container.Input{Folder: "rsx", GroupPath: <agent dir>, Topic:
   thread, SessionID: r.sessions[thread], Prompt: vantage + "\n---\n" +
   text, ExternalMCP: true}` with a minimal `core.Config` (Name, Image,
   IdleTimeout, app-src dir — exact minimal field set verified at
   execution).
3. `out := container.Run(...)` — blocks for the turn (seconds; the pane's
   existing busy state covers it, `ui/news_view.go:726-733`).
4. `r.sessions[thread] = out.NewSessionID`; emit `Reply{thread,
   out.Result}` or `Failed{thread, out.Error}` — never fabricated content.

Selection: `RSX_TERM_ASSIST=local:<agent-dir>` → Runner;
`RSX_TERM_ASSIST=http(s)://…` → Client; unset → nil backend, byte-identical
offline placeholder (keeper invariant, `rsx-term/CLAUDE.md`). Dependency
shape per §1.1(a): the arizuko import is compile-time only, gated behind
the env opt-in like every dial site.

What the unchanged runner gives us for free, per turn: container isolation
+ lifecycle (idle timeout, graceful stop/kill), the folder-as-`$HOME` mount,
PERSONA.md/SYSTEM.md pickup, settings.json seeding that PRESERVES our
entries (`seedSettings` round-trip, `runner.go:764-855`), the socat MCP
bridge wiring, session resume (`SessionID` in → `NewSessionID` out), env
credential injection (`readSecrets`, `runner.go:261`), and audit/log lines.
rsx-term supplies only data: a directory, a prompt string, a socket.

"Thread" near-term = one terminal-session-scoped id per chat entry;
resume-across-restarts is just persisting the `thread→sessionID` map — a
follow-up, not MVP.

## 3. Channel 1 — per-message context = the user's VANTAGE

Chat is reachable from EVERY screen — by **keypress, never mouse**
(keyboard-only keeper invariant, `rsx-term/CLAUDE.md`): one new `binding`
row (`actOpenChat`, e.g. `a`) in the existing keymap table
(`ui/keymap.go:58-64` — `binding{action, key, alts, help, hint, danger}`),
which already drives dispatch, the per-screen hint line, and the `?` help
overlay from one source of truth (`keymap.go:10`), so the chat key lands in
hints/help/rebinds with zero new mechanism. Discoverability requirement:
every screen renders its **available keys as a legend at top-right**,
generated from the same keymap `hint` fields — extend the existing
hint-line renderer to emit a compact corner legend per screen (a render
change in `view.go`, not a new UI system), so the chat key and each
screen's verbs are always visible.

Pressing the chat key switches to the chat pane and attaches **where the
user was looking**. A positions question arrives tagged from the positions
screen; a depth question from the book. For now, per the founder: **rich
and full, don't over-engineer trimming** — send the full state, tagged with
the vantage, so later view-tailored trimming is a provider change, not a
protocol change.

Block shape (generalizes today's `Render`, `assistant/prompt.go:59-84`,
which handles only news/freeze handoffs):

```
[RSX VANTAGE]
screen: book            # book|news|positions|accounting|...
focus: PENGU ladder, cursor 41250 / frozen row "~10s window"
                        # or: selected headline, positions row, ...
[RSX STATE]             # rich/full for now, same honesty rules
market: rsx · PENGU  mid 41258.5  at 2026-07-11T14:32:07Z
asks: ...  bids: ...    # top-N per side
position: +8  entry 41180  uPnL +12.40
open orders: bid 41200×5
fills this session: 3
news: <selected headline, if any>
```

- **Owner/assembly:** `rsx-term/assistant/vantage.go` — pure
  `RenderVantage(v Vantage) string`, unit-tested like `prompt.go`; the UI
  fills `Vantage` from what it already folds (`assistSnapshot`,
  `ui/news_view.go:771-797`; fills via `position.ApplyFill`,
  `ui/update.go:103-110`) plus the current screen/focus. The existing
  `news.AssistantContext` (`news/context.go:36-54`) becomes one input
  (the news/freeze vantage) rather than the only shape.
- **Injection:** prepended to `Input.Prompt` every turn. Verified: the
  runner has no other per-turn context channel — the platform's
  `<autocalls>` block is routd-side and hardcoded
  (`routd/prompt.go:136-148`), and we don't run routd. Content-prepend IS
  the channel, and since Claude Code's own session carries history, the
  agent sees vantage evolution across turns naturally.
- **Honesty + bounds:** unchanged keeper rules — dash unknowns, label
  frozen rows as aggregate windows (`~`), timestamp every block, never
  fabricate (`notes/honesty.md`). "Rich and full" still means bounded:
  top-N levels (today `topLevels=3`, `prompt.go:20`; raise to ~5), one
  screen's focus detail, ≤ ~30 lines. The vantage is *what the user sees*;
  it is deliberately NOT the agent's data ceiling — that's §4's job.

## 4. Channel 2 — MCP: fetch EVERYTHING + control EVERYTHING

The big one. rsx-term itself becomes an MCP server; the containerized agent
reaches it through mechanisms the runner ALREADY ships — no new arizuko
code path. "The model just knows all screens — perhaps more than the user
sees."

**Serving: in-process, not sidecar.** The state lives in the bubbletea
model; a sidecar would need its own feed and re-fold everything. rsx-term
serves a unix socket with the same library arizuko uses
(`github.com/mark3labs/mcp-go`, `ipc/ipc.go:29-30`) from a named goroutine
in `rsx-term/assistant/mcpserver.go`, behind the same env opt-in.

**Wiring — two existing runner mechanisms, use the first:**

1. *Primary: `Input.ExternalMCP: true` + rsx-term listens on the gated
   socket.* An existing public `Input` field whose documented contract is
   exactly this: "the MCP socket is owned by the caller … skip the
   in-container ServeMCP, just mount the ipc dir" (`runner.go:104-106`;
   skip logic `runner.go:231`). rsx-term listens at
   `groupfolder.IpcSocket(ipcDir)` before spawning; in-container Claude
   Code connects via the already-seeded `mcpServers.arizuko` socat entry
   (`runner.go:838-843`) and — because that entry is marked
   `alwaysLoad: true` (`ant/src/mcp-servers.ts`) — sees the full `rsx_*`
   tool surface EAGERLY on every turn, which is what "the model just knows
   all screens" needs. Accepted nit: the server alias reads "arizuko"
   in-container; tool names (`rsx_*`) stay ours.
2. *Alternative (kept in the pocket): a socket inside the mounted folder +
   an operator `mcpServers` entry.* `seedSettings` round-trips
   `.claude/settings.json` and preserves foreign `mcpServers` entries
   (`runner.go:764-855`, research-C §3 "4th path"); the group folder is
   `$HOME` in-container, so `mcpServers.rsxterm = socat STDIO
   UNIX-CONNECT:$HOME/.rsx/mcp.sock` reaches a socket rsx-term binds inside
   the agent dir. Works even without the arizuko Go dep — but third-party
   servers load DEFERRED (Tool Search) rather than eagerly, so the tools
   are discoverable, not omnipresent. Use only if (1) ever misbehaves.

(`connectors.toml [[mcp_connector]]` is NOT applicable near-term: connectors
are spawned/proxied by routd (`ipc/connector.go`), which is dropped.)

**Concurrency bridge (the one real design problem):** MCP handlers run on
socket goroutines; the model folds on the tea update loop. Two paths:

- *Read tools* → a `StateMirror`: after each fold the update loop publishes
  an immutable snapshot (atomic pointer swap) of book/positions/orders/
  fills/news/screen. Handlers read the pointer — no locks on the fold path,
  no stale-lock hazards, always self-consistent.
- *Control tools* → `p.Send(msg)` into the tea program — the exact path
  keystrokes take, so every existing guard (fat-finger hard block, screen
  state machine) applies to the agent identically. Handler waits on a reply
  channel with a short deadline; timeout returns an honest error.

**Read surface (per screen, JSON out, all from the terminal's own folded
state — which kills the old gateway-replay caveat: positions/fills are
served from the same `ApplyFill` folds the trader sees, the single honest
source):**

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

**Control surface (start narrow, widen deliberately):**

| Tool | Action |
|---|---|
| `rsx_goto_screen(screen)` | navigate: book/news/positions/accounting/chat |
| `rsx_focus_symbol(symbol)` | switch the active market |
| `rsx_set_cursor(px)` / `rsx_freeze_row(px)` | drive the ladder cursor / book-microscope freeze |
| `rsx_propose_order(side, px, qty)` | **confirm-gated**: renders a pre-filled order ticket in the terminal; the TRADER presses the key to send. Tool returns `"pending trader confirm"`, never a fill. |

Order entry never executes agent-side — the fat-finger cap and the
keyboard-confirm stay in the terminal path (keeper: hard block, not
warning). Whether `rsx_propose_order` exists at all is founder question 1.

**Push/pull dedup rule:** vantage = what the user is looking at, frozen and
labeled, one per message; MCP = anything, live, agent-initiated. The
vantage block carries no promise of completeness — the CLAUDE.md (§5) tells
the agent "for anything current or off-screen, call the tools".

## 5. Channel 3 — the standing setup: the agent folder's CLAUDE.md

The general RSX know-how — how the agent *thinks* — is not per-message; it
is the group-folder setup the runner mounts as `$HOME` every spawn:

```
<agent-dir>/                      # RSX_TERM_ASSIST=local:<agent-dir>
  CLAUDE.md                       # THE standing instructions (below)
  PERSONA.md                      # voice/register (exists: PERSONA.main.md);
                                  # read at spawn, runner.go:491-499
  .claude/skills/rsx-*/SKILL.md   # optional deep topics, on-demand
  MEMORY.md, .diary/, work.md     # the agent's own accumulation — untouched by us
```

**CLAUDE.md content (rsx-term-authored, the agent's operating manual):**
identity + honesty rules ("not in the snapshot → say so; dash unknowns; a
frozen far row is an aggregate window"); the `[RSX VANTAGE]`/`[RSX STATE]`
block contract; the tool map (which `rsx_*` tool answers which question;
vantage for "what the user sees", tools for "now / off-screen"); fixed-point
conversion (tick/lot, raw i64); reading the ladder (live ~100ms bins vs
time-weighted windows); position folding + uPnL formula; funding mechanics
(zero-sum per symbol); control etiquette (navigate freely, never propose an
order unasked). Deeper single topics can graduate to
`.claude/skills/rsx-*/SKILL.md` (pure markdown, front-matter-matched,
on-demand — research-C §2), but start with ONE CLAUDE.md; split only when
it outgrows a few hundred lines.

**Authoring/ownership:** source of truth in the RSX repo —
`rsx-term/assistant/agentdir/` (CLAUDE.md, PERSONA.md, skills) — installed/
synced by `make assist-agent-dir AGENT_DIR=...`. rsx-term writes it; the
runner mounts it; arizuko never needs to understand a word. One wrinkle to
verify at execution: `seedSkills` copies the full platform skill set
(90+ dirs) from the arizuko checkout into `.claude/skills` every spawn
(`runner.go:992-1000`) — likely noise for a desk agent; see backport
candidate B3.

## 6. Composition — one worked trace

**T+0, from the book.** Trader freezes a PENGU row, hits the chat key,
types "is this wall real?". rsx-term renders the book-vantage block
(screen: book, frozen row labeled `~10s window`, full state) + text →
`Runner.Ask` → `container.Run` spawns Claude Code on the agent folder;
CLAUDE.md primes it; it answers from the vantage. `Output.NewSessionID`
stored for the thread. *(Channels 1+3.)*

**T+2min, follow-up.** "what about now?" — new turn, same thread
(`SessionID` resumes; history is Claude Code's). Fresh vantage block shows
the book has moved; the agent compares, and calls
`rsx_get_orderbook(depth=10)` for depth below the vantage's top-5 — served
from the StateMirror, microseconds, live. *(1+2+3.)*

**Same turn, agent acts.** "show me" → agent calls
`rsx_goto_screen("book")` + `rsx_set_cursor(41250)`; the tea loop applies
them exactly as keystrokes; the trader watches the cursor land. Any order
would stop at a confirm ticket. *(2, control.)*

## 7. Reuse → extend → backport (no fork, ever)

Near-term is **zero arizuko changes** — everything above rides public,
already-shipped contracts. The list below is strictly LATER, optional
upstream PRs: places where a small hook would make the embedding cleaner,
noted now so they land as tidy contributions instead of divergence:

- **B1 — pin the embedder contract.** We rely on `container.Run` +
  `Input.ExternalMCP` + `Output.NewSessionID` as a library API. Upstream
  PR: document "library mode" in arizuko's EXTENDING.md and mark those
  types' stability, so routd/runed refactors don't silently break
  embedders.
- **B2 — progress callback.** `Run` discards stdout and only counts `[ant]`
  stderr lines (`Output.MessageCount`); the local backend therefore has no
  streaming status (pane shows busy until the turn ends — the HTTP backend's
  `Status` frames don't exist here). Upstream PR: `Input.ProgressFn
  func(line string)` invoked per `[ant]` line — gives every embedder (and
  runed itself) live progress. Until then: busy-marker only, which the pane
  already renders honestly.
- **B3 — optional platform-skill seeding.** A cfg flag to skip
  `seedSkills`' full platform copy for embedder folders (lean desk agent).
- **B4 — the vantage/tool-map prompt patterns** we prove locally backport
  later as the PERSONA/skill content of the full-platform folder — the
  LATER layer (routd/webd/threads/social, PLAN.md Phases 0-3) swaps
  `Runner` for the existing HTTP `Client` behind the same `Backend`
  interface; topics become routd sessions; the terminal-MCP moves behind a
  connector or an upstream "delegate this folder's gated.sock" hook
  (B1's natural sequel).

## 8. Phased build order (near-term target first)

1. **A — runner backend.** `Backend` interface extraction +
   `assistant/runner.go` (`container.Run`, session map, env selection) +
   agent-dir template with CLAUDE.md/PERSONA.md + `make assist-agent-dir` +
   an empty mcp-go server on the socket (so the bridge connects cleanly).
   Done-check: multi-turn chat from the pane, fully local, offline goldens
   untouched.
2. **B — vantage provider + chat key.** `vantage.go` with per-screen focus
   tagging (rich/full payload); the `actOpenChat` binding row in the keymap
   table; the keymap-generated top-right key legend on every screen.
   Depends on A only for end-to-end testing; goldens extended, not
   rewritten.
3. **C — MCP read tools.** StateMirror + the eight read tools. Independent
   of B; after A.
4. **D — MCP control verbs.** Navigation set via `p.Send`; then (founder
   gate) `rsx_propose_order` with the confirm ticket.
5. **E — later layer.** Full arizuko platform per PLAN.md; swap backend to
   `Client`; backports B1-B3 land upstream when touched.

## 9. Open questions for the founder (3)

1. **Control ceiling:** does `rsx_propose_order` (confirm-gated ticket,
   trader presses the key) belong in the first control set, or is control
   strictly navigation/focus until further notice? Recommendation: ship
   navigation first, add the confirm-gated ticket in D2 — never an
   unattended execute verb.
2. **Agent folder lifecycle:** one per-machine folder (`local:~/.rsx/agent`,
   memory/diary accumulate across sessions and desks) or per-desk folders?
   Recommendation: singleton — the accumulated MEMORY.md is half the value;
   revisit if multi-desk arrives.
3. **Lean vs full skill seed:** is B3 (skip platform skills for the desk
   agent) worth being our first upstream PR, or do we tolerate the 90+
   platform skills in the folder near-term? Recommendation: tolerate now,
   PR when we first observe skill-match noise in replies.
