# PLAN — rsx-term chat pane → real agentic assistant via arizuko

Sprint: `.ship/45-ARIZUKO-LLM`. Inputs: research-A-deploy.md, research-B-contract.md,
research-C-runtime-tools.md, research-D-auth-social.md (all in this dir, cited as A/B/C/D below).

## Goal

The terminal's LLM pane (today an honest placeholder,
`rsx-term/ui/news_view.go:509` "no model wired") becomes a real assistant:
the existing handoff (`news.AssistantContext`, `rsx-term/news/context.go:36`)
plus a positions/fills summary is POSTed to a locally deployed arizuko, a
Claude Code turn runs in a Docker container, and the reply streams back into
`viewLLM`. Threads = arizuko `topic` strings; sessions = arizuko's
`(group_folder, topic)` sessions table (B:169-179); social = one route row
exposing the same agent folder on Bluesky (D:186-193). Zero arizuko-core
changes: config + PERSONA.md + connectors.toml + one small binary we own.

## MVP definition

**MVP = Phase 0 + Phase 1.** A trader selects a headline (or freezes a book
row), presses enter, and chats about it — the agent sees the frozen book,
mid, headline/note, and the trader's positions/fills/open orders serialized
into the prompt, and the conversation persists per thread across turns. That
already satisfies "useful to chat about the trading experience, connected to
data and market and past trades" — the market data is a snapshot, which is
the honest thing to hand an assistant anyway (the pane already froze it,
deep-copied, at handoff: `context.go:49-53`). Phase 2 (agent-initiated live
queries via an MCP connector) upgrades snapshot-chat to a live-queryable
agent and is the first fast-follow, not MVP. Phase 3 (Bluesky) is a demo.
Live turns are gated on the founder supplying `ANTHROPIC_API_KEY` (env facts:
none set); every build/test step below is not.

---

## Phase 0 — local arizuko, hello-world turn proved

**Outcome:** `authd+routd+runed+webd` running under `PREFIX=~/.arizuko`; one
agent folder with an RSX persona; a CLI turn and an HTTP `/chat/{token}` turn
both return a reply.

Steps (commands per A:189-217, B:249-313):

```bash
# 1. build (Go 1.26 host toolchain covers arizuko's 1.25.5 via GOTOOLCHAIN=auto)
cd /home/onvos/app/arizuko && make build && sudo make images

# 2. create instance — /srv/data is root-owned, so PREFIX everywhere
export PREFIX=~/.arizuko
./arizuko create rsx            # auto-generates .env AUTH_SECRET/SECRETS_KEY (A:64-69)
                                # seeds default group "main" (A:84-87)

# 3. ~/.arizuko/arizuko_rsx/.env additions (A:71-82):
#      ASSISTANT_NAME=RSX Assistant
#      ANTHROPIC_API_KEY=sk-ant-...      # ← founder-supplied; the only gate
#      WEB_PORT=8095                     # emits webd+proxyd+vited (A:78-79)
#      PROFILE=web                       # skip timed/dashd/davd/onbod (A:79-82)
#      ONBOARDING_ENABLED=0              # belt+braces; not needed for pre-provisioned local (D:294-315)

# 4. persona — groups/main is the agent's $HOME (C:9-13); PERSONA.md read
#    every spawn via readOptional (C:14-19). mkdir -p in case the dir is
#    created lazily at first spawn.
mkdir -p ~/.arizuko/arizuko_rsx/groups/main
$EDITOR ~/.arizuko/arizuko_rsx/groups/main/PERSONA.md   # sketch below

# 5. run + hello world (CLI is zero-config ingress, A:211-217)
./arizuko run rsx &
./arizuko send rsx main "hello" --wait

# 6. mint the HTTP route token once (B:203-229) and prove the HTTP path
curl -sS -X POST http://localhost:<routd-port>/v1/route_tokens/chat \
  -H 'Content-Type: application/json' -d '{"owner_folder":"main"}'
# => {"token":"...","url":".../chat/<token>/",...}
curl -N -X POST "http://localhost:8095/chat/$TOKEN" \
  -H 'Content-Type: application/json' -H 'Accept: text/event-stream' \
  -d '{"content":"ping","topic":"t-smoke"}'          # SSE reply (B:280-295)
```

PERSONA.md sketch (prose, front-matter `name`/`description` per C:14-19):

> name: RSX Assistant. A terse trading-desk analyst inside the RSX terminal.
> Each handoff opens with a `[RSX CONTEXT]` block: origin (news headline or
> book freeze), venue/symbol, timestamp, mid, frozen bid/ask levels, and the
> trader's positions/fills/open orders. Treat that block as ground truth for
> "right now"; never invent prices or fills — say "not in the snapshot" when
> asked beyond it. Short answers; basis points and raw levels over adjectives;
> no generic risk-disclaimer boilerplate.

**Done-check:** step 5 prints a reply; step 6 streams an `event: message`
frame with `role:"assistant"`. Record the minted token + webd host port in
`.ship/45-ARIZUKO-LLM/` for Phase 1.

**Verify-at-execution notes (not open decisions):**
- If the compose wiring sets `AUTHD_URL` on routd, token minting needs a
  bearer with `routes:write` — exchange `AUTHD_SERVICE_KEYS` via
  `POST /v1/service-token` (B:231-245); otherwise the endpoint is open
  (`verify==nil`, B:216-229).
- routd and webd both default to :8080 internally (B:253-255) — read the
  generated docker-compose.yml for the actual host ports.

---

## Phase 1 — wire rsx-term's LLM pane to arizuko (the MVP increment)

**Outcome:** with `RSX_TERM_ASSIST` set, enter on a headline/frozen row opens
the pane, the serialized context posts to `/chat/{token}`, the reply streams
into `viewLLM`, and typed follow-ups continue the same thread. Unset, the
terminal is byte-identical offline placeholder — goldens untouched.

### New package `rsx-term/assistant/`

- `client.go` — `Client` with `Enabled() bool`, `Ask(topic, content string)`
  (non-blocking; queues), `Events() <-chan any`. One named goroutine
  `streamTurn` per Ask: `POST <url>` with `Accept: text/event-stream`,
  body `{"content":..., "topic":...}` (B:21-48), parse SSE frames with
  stdlib `bufio.Scanner` (no new dependency), emit typed msgs:
  `Reply{Topic, Text}` (role assistant frames, B:110-116),
  `Status{Topic, Text}` ("⏳" status frames, B:117-120),
  `Failed{Topic, Err}` (dial/HTTP/timeout). Idle cutoff ~180s (cold container
  can take tens of seconds, A:239-244). Mirrors the `TreeOfAlpha` discipline:
  constructor does no I/O, the goroutine is named, failures are honest
  messages never fabricated content (`news/treeofalpha.go:44-51` precedent).
- `prompt.go` — `Render(ctx news.AssistantContext, snap Snapshot) string`:
  the `[RSX CONTEXT]` block — origin label, venue·symbol, ts, headline
  (text/source/tier) or freeze note, mid, top-N bid/ask levels (px×qty), then
  `Snapshot` = plain struct the model fills (net position, entry, uPnL, open
  orders px/qty/side, session fill count — all client-folded state the
  terminal already has, `ui/update.go:102-110`). Pure function, unit-tested;
  keeps the handoff UI-agnostic (keeper invariant, `rsx-term/CLAUDE.md`).
- `client_test.go` / `prompt_test.go` — httptest SSE server; no live dials.

### Changed files

- `rsx-term/main.go` — `assistSource()` beside `newsSource`
  (`main.go:161-171` pattern): reads `RSX_TERM_ASSIST`; unset → nil (the ONLY
  dial gate); passes client into `ui.Config`; `go drainEvents(p,
  assist.Events())` reusing `drainEvents` (`main.go:452-456`).
- `rsx-term/ui/model.go` — `Config.Assist` (nil = offline, like
  `Config.News` `model.go:57-59`); model fields: `assistTopic string`,
  `assistLog []assistLine{role,text}`, `assistInput string`,
  `assistBusy bool`.
- `rsx-term/ui/news_view.go` — `handoffToAssistant`/`freezeToAssistant`
  (`news_view.go:413,442`) additionally: mint a fresh topic (new handoff =
  new thread), call `Assist.Ask(topic, Render(ctx, m.assistSnapshot()))`,
  set `assistBusy`. `viewLLM` (`news_view.go:506`): context block unchanged;
  reply pane renders the transcript + `> input_` line when enabled, exact
  current placeholder when disabled. `handleLLMKey` (`news_view.go:588`)
  grows the typing grammar (below).
- `rsx-term/ui/update.go` — fold `assistant.Reply/Status/Failed` into
  `assistLog`/`status`; input-capture branch for screenLLM before keymap
  lookup, mirroring `newsSearch` capture (`update.go:153-155`): printable →
  `assistInput`, backspace edits, enter → `Ask(assistTopic, input)`, esc →
  clear/back; tab/shift+tab still cycle views (checked first).
- `rsx-term/ui/keymap.go` — LLM screen help entries ("type to chat · enter
  send · esc back").
- Docs: `SCREENS.md` (LLM screen), `README.md` env table
  (`RSX_TERM_ASSIST`), new `notes/assistant.md` (Problem → Fix → Cost: why
  arizuko + route token, why SSE, why snapshot-in-prompt not live tools yet).

### Env vars

- `RSX_TERM_ASSIST` — the full chat URL including the minted token,
  e.g. `http://127.0.0.1:8095/chat/<token>`. Unset (default) = fully offline,
  consistent with the single-var opt-ins `RSX_TERM_NEWS` / `RSX_TERM_VENUE`.
  One variable, no second knob; timeouts/topic are internal.

### Topic (thread) scheme

`t-<unixms>-<venue>-<symbol>` generated client-side at handoff — arizuko
never returns its auto-topic, so the client MUST generate and resend its own
(B:145-153). Each handoff starts a new thread; typed follow-ups reuse
`assistTopic`, so routd resolves the same `sessions` row and the agent keeps
context (B:297-308). Thread list / history rehydration
(`GET /chat/{token}/{topic}/messages`, B:157-159) is a follow-up, not MVP.

### Invariants preserved

Offline-by-default: nil client, zero dials, named goroutine only behind the
env gate. Never fabricate: only received SSE text renders; waiting shows a
busy marker in the status line, not content; failures render as
`assistant unreachable — <err>`. Goldens: DOM/book renders untouched;
`viewLLM` offline output byte-identical; `news_view_test.go` extended, not
rewritten.

**Done-check:** offline — `go test -race ./...` + goldens green with no env
set. Online — start Phase 0 stack, `RSX_TERM_ASSIST=... RSX_TERM_STREAM=1
RSX_TERM_NEWS=1 go run .`, hand off a headline, get a reply that references
the frozen mid; type "what's my position?" in the same thread and get an
answer sourced from the snapshot block.

---

## Phase 2 — RSX MCP connector (live, agent-initiated queries)

**Outcome:** the agent can pull the CURRENT book/trades itself instead of
relying on the handoff snapshot. Zero arizuko-core change: one
`[[mcp_connector]]` block + one binary we own (C:166-184, 213-243).

- **Binary:** `rsx-term/tools/rsx-mcp/` (package main inside the existing Go
  module — reuses `conn`'s gateway/md WS + protobuf decoding and `wire`
  types; `tools/` already hosts `glyphbank`). Speaks MCP over stdio:
  `tools/list` advertises `rsx_get_orderbook`, `rsx_get_trades`;
  `tools/call` dials the marketdata WS on localhost, takes one snapshot,
  returns JSON, exits. Read-only by construction — no order-entry tool, ever.
- **Positions/fills tools (`rsx_get_positions`, `rsx_get_fills`) are
  conditional:** position is client-folded from fills (`ui/update.go:102-110`);
  a per-call connector only sees fills from connect-time on unless the
  gateway replays on connect — verify first. If not, positions/fills stay
  Phase-1 snapshot-only until RSX grows a read-only query surface (C's
  option B, C:245-255 — explicitly out of scope here; RSX-side changes: none).
- **Config:** append to `~/.arizuko/arizuko_rsx/connectors.toml` (C:226-233):

  ```toml
  [[mcp_connector]]
  name         = "rsx"
  command      = ["/home/onvos/app/rsx/rsx-term/tools/rsx-mcp/rsx-mcp"]
  env_template = { RSX_MD_URL = "ws://127.0.0.1:8180" }
  scope        = "per_call"
  ```

  Runs on the HOST next to routd → reaches RSX over localhost, no container
  egress/crackbox concerns (C:236-241). Grant `mcp:rsx_*` only to the agent
  folder (C:178-184).
- PERSONA.md gains two lines: prefer `rsx_get_orderbook` for "right now",
  the `[RSX CONTEXT]` block for "at handoff".

**Done-check:** ask the agent "what's the book on PENGU right now?" with a
stale handoff — reply cites fresh levels matching the terminal, and the tool
call appears in the turn log.

---

## Phase 3 — social: the same agent on Bluesky (demo)

**Outcome:** mentioning the bot on Bluesky gets a reply from the same folder
(same memory/persona), proving "social through arizuko".

- Copy `template/services/bskyd.toml` into the instance's `services/` dir
  (A:36-40); set `BLUESKY_IDENTIFIER`/`BLUESKY_PASSWORD` (D:172-177); bskyd
  self-registers at boot (D:163-178).
- One operator command: `./arizuko group rsx add bluesky:user/<did> main`
  (D:186-193) — attaching an existing folder is just another route row
  (D:239-244).
- Threads stay naturally separate: Bluesky topics are post AT-URIs, terminal
  topics are `t-...` (D:270-287) — no bleed between social and desk threads.
- Check the cost/budget gate before enabling unattended public traffic
  (A:177-182, budget follow-up flagged there).

**Done-check:** a mention on Bluesky yields an on-persona reply; terminal
threads unaffected.

---

## Open decisions for the founder (3)

1. **Anthropic credentials.** `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN`
   into the instance `.env` (A:161-176). Blocks every live turn (Phase 0
   done-check onward); blocks no build/test work. Which, and whose account?
2. **Is Phase 2 in the first cut?** Recommendation: no — ship 0+1
   (snapshot-chat with threads), then the connector as fast-follow. Say the
   word if "agent can query live" must be in the demo from day one.
3. **Docker-per-turn latency.** Mandatory in arizuko — no exec/dev mode
   (A:98-126); expect seconds per reply, cold start possibly tens of seconds
   (A:239-244). Fine for chat (off the trading hot path; SSE busy states keep
   the pane honest). Accept for MVP, or ask for a persistent-session "special
   deployment" later — which WOULD mean arizuko changes, against the
   "manage without" guidance.

## Risks / constraints

- **Latency is seconds, not µs** — chat-pane only; nothing touches the
  <50µs order path. The pane must show busy/failed states honestly (keeper:
  never fabricate).
- **No arizuko-core edits** — everything here is instance config, PERSONA.md,
  connectors.toml, a route row, and a binary in the RSX repo. If any step
  seems to need an arizuko patch, stop and re-plan.
- **RSX-side blast radius: rsx-term only** — one new Go package, edits in
  `main.go`/`ui/{model,news_view,update,keymap}.go`. No Rust crate changes;
  `rsx-cast` frozen and untouched.
- **Phase-0 unknowns to verify by reading generated output, not assuming:**
  routd authz mode (open vs service-token, B:216-245), host-port mapping
  (B:253-255), whether `groups/main/` exists before first spawn (C:53-61).
- **Cost:** every turn bills the API key; the social channel makes traffic
  unattended — read `cmd/arizuko/budget.go` gating before Phase 3 (A:177-182,
  249-253).
- **Goldens/offline CI:** default path makes zero network calls by
  construction (nil client), so `TestDomViewGolden`/`TestBookViewGolden`/
  `TestDefaultViewUnchanged` and `go test -race ./...` stay the gate.
