# Research C — arizuko runtime: shaping an agent + custom tools for RSX data

Repo studied: `/home/onvos/app/arizuko` (read-only). Goal: figure out the
least-effort, cleanest way for a trading-assistant agent folder to query
live RSX market data, order book, and the user's trades/fills/positions.

## 1. Persona/folder layout

A "group folder" (`groups/<folder>/`) is the agent's whole world — bind-
mounted as the container's `$HOME` (`container/runner.go:512-520`,
`m = append(m, volumeMount{Host: hp(cfg, groupDir), Container: containerHome})`).

Contents, read by `container/runner.go` on every spawn:

- **`PERSONA.md`** — front-matter (`name`, `description`) + prose voice/tone
  instructions, read via `readOptional(filepath.Join(groupDir, "PERSONA.md"))`
  (`container/runner.go:491-499`). Auto-migrates the legacy `SOUL.md` name.
  Template: `/home/onvos/app/arizuko/ant/PERSONA.md` — sets calm/precise
  voice, greeting format, memory discipline, uncertainty handling.
- **`SYSTEM.md`** — optional extra system prompt, also read via
  `readOptional` (`container/runner.go:499`).
- **`.claude/skills/`** — per-group skill copies, seeded from the platform
  skill set at spawn (`seedSkills`, `container/runner.go:992-1000+`); an
  operator can drop group-specific skills here too (merge-base tracked so
  platform updates don't clobber local edits).
- **`.claude/settings.json`** — Claude Code settings, **rewritten every
  spawn but round-tripped through the existing file first**
  (`seedSettings`, `container/runner.go:764-855`): reads current
  `settings.json`, only overwrites the keys it owns (`env`, `permissions`,
  `sandbox`, `mcpServers.arizuko`), and preserves anything else — critically
  including any operator/agent-added third-party `mcpServers` entries.
- **`MEMORY.md`** / **facts/** / **`.diary/`** / **`work.md`** — accumulated
  knowledge injected as prompt annotations at spawn: diary
  (`diary.Read(groupDir, 14)`, `container/runner.go:480`), `work.md`
  (`container/runner.go:482-485`), user-context XML
  (`router.UserContextXml`, `container/runner.go:487`). These are files the
  agent itself reads/writes via its normal filesystem tools — no special
  API.
- **`media/`** — per-group scratch dir for attachments, auto-created.
- **`ipc/<folder>/gated.sock`** — NOT inside the group folder; lives under
  `cfg.IpcDir`, bind-mounted into the container at `/run/ipc`
  (`container/runner.go:546-553`). This is the MCP bridge socket (§3).
- **ACL** — not a file in the folder; it's DB rows (`grants` table) keyed
  by folder path prefix, managed via `arizuko group <inst> grant/ungrant`
  or the `add_acl`/`remove_acl` MCP tools. Grant rules are what filter which
  MCP tools (including custom ones) a given folder's agent actually sees
  (`grantslib.MatchingRules(rules, name)`, `ipc/ipc.go:906-912`).

### Creating + registering a new agent folder

`arizuko group <instance> add <jid> <folder> [--product <name>]`
(`cmd/arizuko/main.go:361-426`):

1. `container.SetupGroup(cfg, folder, productDir)` (`container/runner.go:940-966`)
   — creates `groups/<folder>/` + `logs/`, optionally copies a product
   prototype (`PRODUCT.md` + seed files under `ant/examples/<product>/`),
   pre-creates web slots, then `seedGroupDir` seeds `.claude/skills` and
   chowns the tree to the container's uid (1000).
2. `gs.SeedDefaultTasks(folder, folder)` — default scheduled tasks.
3. `gs.PutGroupRow(core.Group{Folder: folder, ...})` — registers the group
   in the routing DB.
4. A route row binds the inbound JID (channel identity) to the folder so
   inbound messages land on this agent.

A **product template** (`ant/examples/<name>/PRODUCT.md` + seed files) is
the mechanism for "shape a reusable agent kind" — copy `PERSONA.md`,
`CLAUDE.md`, `facts/`, and (per §4) a pre-seeded `.claude/settings.json`
`mcpServers` block, into every group created with `--product <name>`. This
is the natural home for a "trading-assistant" agent kind.

## 2. Skills

Every skill in `ant/skills/*/SKILL.md` is **pure markdown** — YAML
front-matter (`name`, `description`, `when_to_use`) + prose instructions.
There is no script or executable component; `ls ant/skills/*/` shows every
skill directory contains only `SKILL.md` (verified across `bugs`, `data`,
`acquire`, `channels`, `bash`, `cli`, and 90+ others). A skill does not
call anything itself — it is discovered by the agent (via Claude Code's
skill-search / `/resolve`-style triggering, driven by `description` +
`when_to_use`) and then the agent follows the instructions using its
**existing** tools: Bash, file read/write, and whatever MCP tools are
registered. `EXTENDING.md` calls skills a "File-based" extension point,
extensible by the Agent — confirming skills are pure prompt content, not
code.

Implication for RSX: a `trading` skill would document *how* to call the
RSX MCP tools (which ones exist, what a fill/position looks like, when to
use `get_orderbook` vs `get_positions`) — it cannot itself talk to RSX.
The actual I/O still needs an MCP tool (§3) or a Bash-callable script.

## 3. MCP tools — hosting + bridging + how to add one

### Hosting (host side, in `routd`)

`ipc.ServeMCP` runs one MCP server per group **inside the `routd` process**,
listening on a unix socket at `ipc/<folder>/gated.sock`
(`ipc/README.md` "Purpose"; `ipc/ipc.go:891-897` `buildMCPServer` +
`server.NewMCPServer("arizuko", "1.0")`). Every registration goes through
one of a handful of wrapper functions inside `buildMCPServer`:

- `registerRaw(name, desc, opts, handler)` — filters by `grantslib.MatchingRules`,
  appends the matching grant rules to the tool's *description* (so the
  model can see its own scope), then `srv.AddTool(...)`.
- `granted(name, desc, opts, handler)` — wraps `registerRaw` with a
  `grantslib.CheckAction` + `authorizeCall` gate before invoking the
  handler (`ipc/ipc.go:930-941`).
- `regSocial(socialAct{...})` — a declarative struct-driven registrar for
  the ~15 chat-verb tools (`like`, `forward`, `quote`, ... — `ipc/ipc.go`
  ~1270-1420), the canonical "when/not-for" description pattern cited by
  `EXTENDING.md`.

### Bridging into the container

Inside the container, Claude Code is told about one MCP server named
`arizuko` in `.claude/settings.json`:

```go
// container/runner.go:838-843
servers["arizuko"] = map[string]any{
    "command": "socat",
    "args":    []string{"STDIO", "UNIX-CONNECT:/run/ipc/gated.sock"},
}
```

`socat` runs *inside* the container, bridging Claude Code's stdio MCP
transport to the host-side unix socket (bind-mounted at `/run/ipc`,
`container/runner.go:546-553`). `SO_PEERCRED` on accept checks the
connecting uid (`ipc/README.md`), and identity (folder, tier) is derived
from the socket *path*, not from anything the agent sends.

The TS agent runtime (`ant/src/mcp-servers.ts`) does one more pass at
container startup: `loadAgentMcpServers` reads `.claude/settings.json`,
strips any stale `arizuko` key, and returns whatever *other* `mcpServers`
entries are present (third-party servers the operator or agent
self-registered); `injectMcpEnv` folds resolved secrets into each server's
`env`, marks `arizuko` `alwaysLoad: true` (eager — core tools every turn),
and leaves third-party servers deferred by default (loaded only via Claude
Code's Tool Search Tool when the model actually reaches for them, per spec
6/A). This is the mechanism that lets a **second, independent MCP server**
live alongside the `arizuko` bridge with zero core-repo changes.

### Three ways to add a new tool (increasing effort, decreasing coupling)

1. **Go handler in `ipc.go`** — new `registerRaw`/`granted`/`regSocial`
   call. This *is* core (`ipc/` is explicitly listed as core, not an
   extension point, in `EXTENDING.md`'s table) — requires a spec + PR to
   arizuko itself. Not appropriate for an integration like RSX.
2. **`[[ext]]` REST-descriptor tool** (`ipc/extcall.go` `ExtTool` +
   `routd/ext.go` `LoadExtProviders`) — a declarative TOML block, **no code
   at all**: `base` URL, one `[[ext.tool]]` per endpoint (`method`, `path`,
   templated `{param}`s in the path, rest as query/body), one `[ext.auth]`
   block (`bearer` / `apikey-header` / `apikey-query` / `basic` /
   `json-body`, referencing a secret-store key). `routd/ext.go:44-80`
   loads built-in providers from `routd/extproviders/*.toml` (see
   `namecheap.toml`, `cloudflare.toml`, `porkbun.toml`, `gandi.toml` as
   templates) plus operator `[[ext]]` blocks appended to
   `<data_dir>/connectors.toml`. `CallExtTool` (`ipc/extcall.go:44-140`)
   does one plain `http.DefaultClient.Do` per invocation, on the **host**
   (`routd` process), not inside the container — so no container egress
   (crackbox) concerns at all. Response is scrubbed of secret values
   before returning to the agent (`scrubSecrets`). Known gap:
   `ExtTool.InputSchema` is never populated by `LoadExtProviders`
   (BUGS.md "ext tools: InputSchema never populated", 2026-06-26, open,
   low severity) — agents currently infer args from the tool
   name/description only, which works for LLM callers but is sloppy.
3. **MCP-subprocess connector** (`ipc/connector.go`, spec `specs/7/Y-...`)
   — for wrapping a *stateful* or non-REST upstream. `[[mcp_connector]]`
   block in `<data_dir>/connectors.toml` declares a stdio command; routd
   spawns it **per call** on the host, does an MCP `initialize` +
   `tools/list` handshake at boot to harvest the tool catalog (namespaced
   `<connector>_<remote_tool>`), and on each invocation spawns the
   subprocess again with secrets rendered into its env
   (`{secret:KEY}` → `env_template`), proxies one `tools/call`, tears the
   subprocess down. This is the right shape when the upstream isn't a
   plain synchronous HTTP GET/POST — e.g. anything over WebSocket, gRPC, a
   local binary protocol, or something needing multi-step client logic.

Both (2) and (3) are visibility-gated the same way as built-in tools: each
registration checks `db.Authorize("folder:"+folder, folder, tool.Scope /
"mcp:"+tool.LocalName, nil)` before announcing the tool to a given folder
(`ipc/ipc.go:1024-1030`, `1044-1050`) — so a connector or ext tool can be
restricted to only the trading-assistant folder via an ACL deny rule on
other folders (default tier allows `mcp:*`, i.e. visible everywhere unless
explicitly denied).

**A 4th path exists that isn't in `EXTENDING.md` but is real and even
lighter-weight:** because `seedSettings` round-trips `.claude/settings.json`
through the existing file and only overwrites its own known keys
(`container/runner.go:770-778`, `838-843`), an operator (or a product
template's seed files) can drop an **arbitrary third-party `mcpServers`
entry directly into `groups/<folder>/.claude/settings.json`** — any
`command`/`args`/`env` Claude Code can spawn. It survives every re-spawn,
is per-folder by construction (it's a file inside that folder), gets
folded with resolved secrets and deferred-loaded by `ant/src/mcp-servers.ts`.
This requires the command to be executable **inside the container**
(so either baked into the `ant` image, or a script dropped into the
group's home dir, which is host-writable and bind-mounted), and any
outbound network call the script makes is still subject to crackbox
per-folder egress allowlisting (`network_allow`/`store/network.go`) — unlike
the ext/connector paths, which run on the host and never touch the
container's network stack.

## 4. Recommended path for RSX market + trade data

RSX has **no REST API** — the wire protocol is length-prefixed protobuf
(order channel) / protobuf marketdata frames over WebSocket
(`specs/2/49-webproto.md`), not a plain HTTP GET/POST surface. That rules
out the declarative `[[ext]]` REST descriptor (§3.2) for talking to RSX
*directly* — there's no synchronous URL to point it at.

Two real options, in order of effort:

**A. MCP-subprocess connector (recommended for MVP and steady state).**
Write one small standalone program (Go, since `rsx-messages`/`rsx-types`
are already Rust crates you could reuse via a thin Rust binary, or a
quick Python/Node script using `rsx-playground`'s existing WS/protobuf
decoding as reference — `rsx-playground/md_wire.py` already decodes the
marketdata feed) that speaks MCP-over-stdio: on `tools/list` it advertises
a handful of tools (`rsx_get_orderbook`, `rsx_get_trades`,
`rsx_get_positions`, `rsx_get_fills`), and on `tools/call` it opens (or
reuses) a WS connection to the RSX gateway/marketdata process, sends the
appropriate query/subscribe-then-snapshot request, decodes one protobuf
response, and returns JSON text. Declare it once in
`<data_dir>/connectors.toml`:

```toml
[[mcp_connector]]
name         = "rsx"
command      = ["/opt/arizuko/rsx-mcp/rsx-mcp"]
secrets      = ["RSX_API_TOKEN"]
env_template = { RSX_TOKEN = "{secret:RSX_API_TOKEN}" }
scope        = "per_call"
```

Runs on the host next to `routd` (this machine also runs RSX, per the
working-directory layout `/home/onvos/app/rsx` beside `/home/onvos/app/arizuko`),
so it reaches RSX's gateway/marketdata over localhost with zero crackbox
/ container-egress configuration. Grant `mcp:rsx_*` only to the
trading-assistant folder to keep the tools out of every other agent's
tool list. Least new surface: no arizuko core edits, no container image
rebuild, one TOML block + one small standalone binary you fully control
(so it can be as read-only/safe as you want — e.g. never exposing an
order-entry tool, only queries).

**B. Give RSX a proper query surface, then use `[[ext]]`.** If a thin
read-only HTTP query endpoint is added to `rsx-gateway` (or a new small
side-process) exposing book snapshot / trade-history / position lookups
as plain JSON GET endpoints, the `[[ext]]` TOML descriptor (§3.2) becomes
usable and is genuinely zero-code on the arizuko side — cleanest long-term
if RSX ever wants a general HTTP query API anyway (useful for the
`rsx-playground` dashboard too). More effort now (new RSX-side code,
crosses into RSX's "Trust boundaries" territory — a new read-only query
API is a new attack surface to design, per RSX's own
`specs/2/47-validation-edge-cases.md` discipline) for a cleaner shape
later.

**Recommendation: start with A.** It needs nothing from RSX's own repo,
composes with arizuko's existing secret broker + per-folder grants, and
the connector binary is a natural place to keep RSX-specific decoding
logic (reusing `rsx-types`/`rsx-messages` if written in Rust) without
touching either codebase's core. Revisit B only if/when RSX wants a
general HTTP query API for reasons beyond this agent.

The 4th path (§3, per-folder `mcpServers` entry baked into the group's
`settings.json`) is viable but strictly worse here: it would run the
connector logic *inside* the agent's Docker container, requiring a
crackbox `network_allow` rule for the RSX host and bundling the RSX
client/decoder into (or bind-mounting it into) the `ant` container image.
No benefit over (A) for this use case since RSX and arizuko share a host.

## 5. Runtime pinning to Claude Code + credential flow

Yes, pinned to Claude Code / the Claude Agent SDK. `ant/src/*.ts` imports
`@anthropic-ai/claude-agent-sdk` directly (e.g.
`ant/src/tool-log.ts:9`), and `ant/entrypoint.sh` compiles/runs that
TypeScript, which drives the SDK's `query()` — there is no
runtime-agnostic abstraction layer; swapping models means swapping the SDK
call, not a config flag. `ant/character.json`/`PERSONA.md` shape behavior,
but the executor is Claude Code end to end.

Credential flow (`container/runner.go:706-732`, `container/secrets_test.go`):

- Operator anchor: `ANTHROPIC_API_KEY` or `CLAUDE_CODE_OAUTH_TOKEN` read
  from routd's own process env (`readSecrets`, `container/runner.go:707-718`).
- Per-folder/user override (BYOA — bring your own Anthropic key): routd
  resolves a folder/user-scoped secret and `mergeSecrets` overlays it onto
  the operator anchor, override wins per key (`container/runner.go:724-732`).
- The resolved key becomes a container env var, picked up by the Claude
  Agent SDK at query time same as any local Claude Code install — no
  special arizuko-side auth negotiation with Anthropic beyond "which key
  goes in the env".

## Sources

- `ant/PERSONA.md`, `ant/entrypoint.sh`, `ant/src/mcp-servers.ts`, `ant/src/tool-log.ts`
- `container/runner.go` (SetupGroup, seedGroupDir, seedSkills, buildMounts,
  seedSettings, readSecrets, mergeSecrets) lines ~480-990
- `container/secrets_test.go`
- `groupfolder/README.md`, `groupfolder/folder.go`
- `ipc/README.md`, `ipc/ipc.go` (buildMCPServer, registerRaw/granted/regSocial,
  connector + ext-tool registration ~1010-1075), `ipc/extcall.go`,
  `ipc/connector.go`
- `routd/ext.go`, `routd/extproviders/*.toml`
- `specs/5/41-ext-mcp.md`, `specs/7/Y-connectors.md` (referenced by EXTENDING.md)
- `EXTENDING.md` (extension-point table, "Adding an MCP connector" section)
- `cmd/arizuko/main.go` (`cmdGroup`, `add` case)
- `BUGS.md` "ext tools: InputSchema never populated" entry
- RSX side: `/home/onvos/app/rsx/specs/2/49-webproto.md` (WS + protobuf,
  no REST), `/home/onvos/app/rsx/rsx-playground/md_wire.py` (existing
  marketdata decoder reference)
