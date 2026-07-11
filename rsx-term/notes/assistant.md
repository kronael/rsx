# Wiring the chat pane to a real agent — arizuko, a route token, and SSE

The LLM screen shipped as an honest placeholder: it froze a real
`news.AssistantContext` at handoff and rendered *exactly* what a wired model
would receive, over a "no model wired" reply pane. Phase 1 makes the reply pane
real without touching the handoff contract, the goldens, or the offline default.

## Problem → Fix → Cost, three decisions

### 1. Why arizuko + a route token (not a raw Anthropic call)

**Problem.** A chat pane needs an agent that keeps per-thread memory across
turns, runs tools, and can later answer on other channels — not a stateless
one-shot completion. Baking a provider SDK, prompt-caching, and a sessions store
into the terminal would drag all of that onto the order-flow codebase.

**Fix.** Point at a locally deployed arizuko instance and POST to its
`/chat/{token}` surface. The URL route-token *is* the whole credential
(`webd`: `/chat/*` stays unauthenticated, the token is the capability), so the
terminal holds one opaque string and no keys. arizuko owns sessions
(`(group_folder, topic)`), the Docker-per-turn Claude Code runtime, the persona,
and — later — the Bluesky channel. Zero arizuko-core change: instance config, a
persona file, and this client.

**Cost it removes.** No provider SDK, no key handling, no sessions table, no
tool runtime in `rsx-term`. The terminal stays a terminal.

### 2. Why the client generates the topic

arizuko auto-generates a topic when the body omits one, **but never returns it**
— so a client that wants a stable, resumable thread must generate and resend its
own. Each handoff mints `t-<unixms>-<venue>-<symbol>` (a new thread per handoff);
typed follow-ups reuse it, so routd resolves the same `sessions` row and the
agent keeps context. Thread history rehydration
(`GET /chat/{token}/{topic}/messages`) is a later follow-up, not MVP.

### 3. Why SSE, and why a snapshot in the prompt (not live tools yet)

**SSE.** One `POST` with `Accept: text/event-stream` streams the reply back on
the same connection — no separate turn-id to poll. Parsed with a stdlib
`bufio.Scanner`, no new dependency. Latency is *seconds* (a fresh Docker
container per turn, cold start possibly tens of seconds) — fine for chat, which
is off the <50µs order path, as long as the pane shows busy/failed states
honestly. An idle-cutoff context (reset on every frame) abandons a wedged turn
without capping a legitimately long stream.

**Snapshot, not live tools.** The pane already froze the book (deep-copied) at
handoff, and the trader's position/fills/open-orders are client-folded state the
terminal already holds. `assistant.Render` serializes both into the prompt. A
snapshot is the *honest* thing to hand an agent for "at handoff"; letting the
agent pull the *current* book itself is Phase 2 (an MCP connector, RSX-side
changes: none). Shipping the snapshot first keeps the increment small and the
blast radius `rsx-term`-only.

## Invariants this had to preserve

- **Offline by default.** One env gate (`RSX_TERM_ASSIST`, the full chat URL).
  Unset → nil client → zero dials → the reply pane is byte-identical to the
  placeholder (`TestAssistReplyLinesOfflineExact` byte-locks it). The only dial
  site is the named `streamTurn` goroutine, gated on `Enabled()`.
- **Never fabricate.** Only received SSE text renders. Waiting is a `⏳` marker
  in the status line, not content. A failure is `assistant unreachable — <err>`,
  an error row, never a fake reply. (`notes/honesty.md`.)
- **Model UI-agnostic.** `assistant.Render(ctx, snapshot)` is a pure function
  with no terminal types — unit-tested without the `ui` layer. `Snapshot` is
  plain data the terminal fills; the prompt's number format is self-contained.
- **Goldens byte-locked.** The DOM/book renders and the offline LLM pane are
  untouched; `news_view_test.go` was extended, not rewritten.
