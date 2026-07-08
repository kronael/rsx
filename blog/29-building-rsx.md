# Building RSX: A Spec-First, Agent-Driven Manual

*The vibe-book. How one person built a derivatives exchange — matching
engine, risk engine, WAL replication, gateway — by writing specs first
and driving a fleet of AI agents through a disciplined loop. This is the
method, not the marketing. If you want to build something serious this
way, this is the playbook.*

The thesis is uncomfortable for both camps: **vibe-coding is not sloppy,
and "serious engineering" is not slow.** The reason most agent-built code
is junk isn't the agent — it's the absence of a contract for the agent to
build against and a judge hostile enough to reject the result. Put those
two things in place and a single person moves at the speed of a team
without shipping a team's worth of bugs.

RSX is the existence proof: ~35k lines of Rust, 800+ tests, a sub-50µs
latency budget, zero heap on the hot path — built solo, agent-driven.
Here's how.

---

## 0. The toolkit: `kronael/tools`

Everything below runs on [`kronael/tools`](https://github.com/kronael/tools)
— a Claude Code configuration (skills, hooks, agents) plus a few CLIs.
Install it:

```sh
git clone https://github.com/kronael/tools /tmp/kronael
cd /tmp/kronael
claude     # then type: install
```

The pieces that matter for building a system like RSX:

| Tool | Role |
|------|------|
| `/ship`    | **outer loop** — specs → components (topological) → completion |
| `/build`   | **inner loop** — plan → parallel workers → judge → commit |
| `/refine`  | finalization pass — build/test → `@improve` → `@readme` → commit |
| `cto-eval` | adversarial **code** audit — verify load-bearing claims, attack scenarios, numeric grade |
| `ceo-eval` | adversarial **demo** audit — boot it, run the flow, inject faults, grade demo-readiness |
| `/diary`   | append the day's decisions to `.diary/YYYYMMDD.md` |
| `/fin`     | finish-mode — re-run the open-items pass; no fake "deferred" |
| `rig`      | detached-HEAD git: `rco` checkout, `rip` push — no branch clutter |
| `dockbox`  | a sandboxed Claude Code container so agents can't touch your host |

You don't need all of it. You need the *loop* (`/ship` → `/build` →
`/refine`), the *judges* (`cto-eval`, `ceo-eval`), and the *discipline*
(specs, gates, git). The rest is ergonomics.

---

## 1. Spec-first, and mean it

**Write the spec before the code.** RSX has 47+ spec files in `specs/2/`,
each numbered and cross-referenced, written before (or alongside) the
crate it describes. This is not documentation theatre. The spec is
load-bearing for three reasons:

1. **It's the contract the agent builds against.** A vague prompt yields
   vague code. `specs/2/21-orderbook.md` says exactly what a price level
   is, what FIFO-within-level means, what events the matcher emits. The
   agent has nowhere to hallucinate.
2. **It's the judge's rubric.** `cto-eval` can't grade "is this good" —
   it grades "does this satisfy the spec." No spec, no objective review.
3. **It's the unit of parallelism.** Components partition along spec
   boundaries, so independent agents own disjoint specs and never collide.

A spec is *executable* — ready to hand to an agent — when it has all six:
deploy target, scope boundary (what's in/out, what's already done),
success criteria (a named test suite + threshold, not "tests pass"),
interface spec (entry points, protocols, I/O), edge-case format, and a
current-state baseline. Miss one and the build plan gets rejected. (This
checklist lives in `blog/README.md` and earned itself by rejecting real
plans.)

The order matters too: write the *transport* spec before the *exchange*
spec, because the exchange is built on it. RSX's `rsx-cast` (the
log-backed reliable-UDP transport) was specced and frozen before a single
matching-engine line existed. See
[Design Philosophy](01-design-philosophy.md) for why 35 docs came first.

---

## 2. The loop: `/ship` → `/build` → `/refine`

The agent workflow is a nested loop, not a chat.

```
/ship (outer)                    scan specs, topo-sort components
  └─ for each component:
       generate build plan  ──▶  /build (inner)
                                   ├─ spawn parallel workers per stage
                                   ├─ judge loop: poll, retry (max 3)
                                   └─ single commit at the end
       update PROGRESS.md
       critique vs spec      ──▶  cto-eval (fix gaps if >10%, max 2 rounds)
  └─ final audit, ship summary
```

**`/ship` is the architect.** It reads the specs, sorts the components by
dependency (transport before exchange, types before everything), and for
each one generates a build plan into `.ship/NN-NAME/`. That directory is
the scratch space: PLAN.md, PROGRESS.md, critique notes. In RSX it's
checked into git as a build log — the audit reports and benchmark sprints
are useful "how this got built" artifacts.

**`/build` is the worker pool.** It parses the plan, spawns parallel
agents — one per stage — and runs a *judge loop*: poll each worker, retry
failures (max 3), isolate errors so one bad worker doesn't sink the
batch. One commit at the end, not a mess of intermediate states.

**`/refine` is the finisher.** Build/test → `@improve` (code quality,
3–5 iterations of DO → CRITICIZE → IMPROVE → VERIFY) → `@readme` (sync
README/ARCHITECTURE/CHANGELOG) → verify → commit `[refined]`.

The rule that makes parallelism safe: **one component-bucket × one
concern per agent, disjoint file sets.** Never two agents in the same
file. For RSX's cross-crate audits, components grouped into ≤4 buckets,
one agent per bucket, each owning its files exclusively.

---

## 3. The judges must be hostile

This is the part everyone skips, and it's the part that makes it work.
**An agent's success report is not evidence.** Agents overclaim. They say
"tests pass" without running them; they say "fixed" when they edited the
wrong file. The single most important rule in the whole method:

> NEVER trust a subagent's success report. Check the diff or the output
> it produced.

So you build adversaries:

- **`cto-eval`** — an adversarial *code* audit. It picks ≥5 load-bearing
  claims and tries to falsify each by reading the code. It runs 3 attack
  scenarios. It assigns a numeric SLA grade. It does not care about your
  feelings or your commit message.
- **`ceo-eval`** — an adversarial *demo* audit. It boots the system, runs
  the user-facing flow, injects ≥3 faults, and grades demo-readiness.
  Surfaces the "looks done, isn't" gaps.
- **Parallel agent audits** — for the big sweeps, four agents assume every
  component is hostile and lies. RSX's test-suite archaeology found 90
  bugs this way: race conditions, resource leaks, timing dependencies,
  wrong assertions. See
  [Parallel Agent Audits](11-parallel-agent-audits.md) and
  [Testing Like the System Wants to Lie](14-testing-hostility.md).

The mental model: the builder agent is optimistic, the judge agent is
paranoid, and *you* arbitrate by reading the diff. Optimism ships
features; paranoia ships *correct* features.

---

## 4. Hard gates, not vibes-as-acceptance

"Vibe-book" does not mean the vibe is the acceptance criterion. The vibe
is how you *move*; the gate is how you *land*. RSX enforces a 4-gate
pipeline — and you never run the last gate directly:

```
make gate           # all four, in order
  gate-1-startup     server imports cleanly
  gate-2-partials    HTMX partials return 200
  gate-3-api         Python API tests pass
  gate-4-playwright  full browser suite (421/421 or it doesn't ship)
```

Tests are tiered by speed so the loop stays fast: `make test` (<5s unit,
every commit), `make wal` (<10s correctness), `make e2e` (~3min, every
PR), `make integration` (testcontainers, real Postgres). The discipline:
**build/test every ~50 lines** — errors cascade, and an agent that wrote
200 lines on a wrong assumption is expensive to unwind.

And the meta-rule from the global wisdom that keeps the whole thing
honest:

> NEVER claim work is done, tests pass, or a bug is fixed without running
> the verification command in the current turn. Confidence is not
> evidence.

---

## 5. Git discipline: detached HEAD + `[section]` commits

Branches are the wrong default for this — they drift, pile up, need
tracking config. The method runs in **detached HEAD**: you commit to a
SHA, the remote owns the branch label, you push when ready (`rig`'s `rco`
/ `rip`). Reflog keeps 90 days; nothing is lost.

Commits are `[section] Message` — `[matching] fix ME-NEXT-SEQ
regression`, `[docs] README: add cookbook`. Never `git add -A`, never
`--amend`, never squash, never `--no-verify`. Small, labelled, honest
commits are how the build log stays readable a month later — and how a
fresh agent (or a fresh *you*) reconstructs intent.

---

## 6. Memory: how the project survives context loss

Agents have no memory between sessions. The fix is filesystem memory:

- **`.diary/YYYYMMDD.md`** — the long-lived shipping log. Decisions,
  milestones, open items. Written via `/diary` after significant work.
  This is *the* record — not `.ship/`, which is scratch.
- **`MEMORY.md`** — cross-session facts about the project and the person.
  Loaded at the start of every session. One line per fact in the index;
  detail in topic files.
- **`.ship/NN-NAME/`** — per-sprint scratch (plans, progress, critiques).
  In RSX, checked in as a build log.
- **`PROGRESS.md`** — per-crate status, regenerated from artifacts
  (`make status-doctor` before any edit).

Start every session by reading the last few diary entries and MEMORY.md.
The agent that knows what was decided yesterday doesn't re-litigate it
today.

---

## 7. The boring-code constraint

The method moves fast, so the code must be *simple* — debugging is twice
as hard as writing, and you want headroom. The constraints RSX holds the
agents to:

- Write code simpler than you're capable of. Clarity over cleverness.
- Copy 2–3 times before abstracting. Premature abstraction prevents
  change; a helper that introduces closures/generics absent from the call
  site is not simpler.
- Explicit enum states, not implicit flags. Validate before persistence.
- Plain functions in modules; structs only for state or dependency
  injection. Information is data, not objects.
- Flat module hierarchies, single import per line (clean diffs), tests in
  `src/<module>_test.rs` next to the source.

Boring code is what lets an agent — or a human six months later — change
one thing without tracking the state of ten others.

---

## 8. Reproduce it

The shortest path to building your own system this way:

```sh
# 1. install the toolkit
git clone https://github.com/kronael/tools /tmp/kronael && cd /tmp/kronael && claude  # → install

# 2. write your specs first — specs/2/, numbered, six-point checklist each
#    transport/foundation specs before the things built on them

# 3. run the loop
#    /ship    → it plans + builds components in dependency order
#    cto-eval → adversarial code audit on each, fix gaps >10%
#    ceo-eval → boot it, inject faults, grade demo-readiness
#    /refine  → @improve + @readme, commit [refined]
#    /diary   → record decisions; /fin → close open items honestly

# 4. gate before you land
make gate
```

That's the whole method. Specs are the contract. The loop is the engine.
The judges are hostile. The gates are hard. Memory survives the context
window. And you — the one person — read every diff and arbitrate between
the optimist and the paranoid.

The impossible thing works quite well, if you build it like this.

---

*Companion reading: [Development Journey](05-development-journey.md) (the
RSX timeline), [The Finalize Round](27-finalize-round.md) (the audit
methodology in detail), [Design Philosophy](01-design-philosophy.md) (why
spec-first). For driving the finished exchange rather than building one,
see the [Operations Cookbook](28-cookbook.md).*
