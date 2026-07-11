# Research D — arizuko auth + social-channel attach

Read-only research against `/home/onvos/app/arizuko`. Feeds a plan for
exposing an rsx-tui trading-assistant agent through arizuko, including a
social channel.

## 1. Auth bootstrap (local)

**Keys/secrets needed to run authd:**

- `DATABASE`/`DATA_DIR` — SQLite DSN for `auth.db` (`authd/main.go:33`).
- `AUTHD_SERVICE_KEY` — authd's own bootstrap secret; it self-mints
  `service:authd` and doubles as a service secret
  (`authd/main.go:161-179`, `loadServiceSecrets`).
- `AUTHD_SERVICE_KEYS` — `principal=secret,...` map letting other daemons
  exchange their bootstrap secret for a `service:<name>` JWT
  (`authd/main.go:163-174`).
- `AUTH_SECRET` is **not** consumed by authd itself — it's the HMAC
  secret for legacy session JWTs and proxyd-signed identity headers,
  used by the `auth/` library elsewhere (`auth/README.md` Configuration
  section). ES256 platform tokens are signed by authd's own P-256
  keypair, persisted in `auth.db` (`signing_keys` table), not by
  `AUTH_SECRET`.
- `GRANTS_URL` (optional) — if unset, every session is empty-scope
  (`authd/main.go:65-72`).
- In practice you don't hand-generate any of this: `./arizuko create
  <name>` auto-generates `AUTH_SECRET` + `SECRETS_KEY` into the
  instance's `.env` (`INSTALL.md:126-131`), and `arizuko run` wires
  `AUTHD_SERVICE_KEY`/`AUTHD_SERVICE_KEYS` per-daemon via docker
  compose.

**Token-issuance flow to post a message:**

There is no single "login and get a token" call for posting — two
distinct paths exist:

- **Daemon/service path** (what channel adapters and internal services
  use): `POST /v1/service-token` with `Authorization: Bearer
  <bootstrap-secret>` and body `{"daemon":"<name>"}`
  (`authd/http.go:139-163`, `handleServiceToken`). authd hash-matches
  the secret against `serviceSecrets`/`serviceGrants`
  (`authd/http.go:29-63` — e.g. `service:bskyd → {"messages:write"}`,
  `service:teled → {"messages:write"}`), mints a 15-minute ES256 JWT
  with `sub="service:<name>"` via `MintForSubject`, and returns it.
  This is the token an adapter attaches to
  `POST http://routd:8080/v1/messages` to deliver an inbound message
  (`routd/server.go:387-390`, `handleMessages` requires scope
  `messages:write`).
- **Human/OAuth path**: authd mounts `/auth/*` (GitHub/Google/Discord/
  Telegram-widget) when `AUTH_BASE_URL` is set; a successful login
  exchanges for an ES256 access+refresh token pair
  (`authd/README.md` "Purpose"). `POST /v1/tokens` is the
  issuer-mint/downscope entry point once a caller already holds a
  valid bearer (`authd/http.go:190-197`, `handleTokens`).
- **Actual posting endpoint**, regardless of path: `POST
  /v1/messages` on **routd** (not authd) with `Authorization: Bearer
  <token>`, scope `messages:write` — this is the same endpoint channel
  adapters and web/local clients (`webd`) hit (`routd/server.go:234`,
  `routd/server.go:387`; also documented in
  `specs/4/1-channel-protocol.md` "Deliver inbound message").

**Principal/identity model:**

- Wire format: `sub` claims like `"user:abc123"`, `"service:routd"`,
  `"agent:atlas/main"` (`auth/README.md` TokenClaims example).
- `X-User-Sub` is a header set by **proxyd** after OAuth completes,
  forwarded downstream to onbod/dashd for the web-login/onboarding UI
  (`onbod/integration_test.go:232-253`), not the JWT `sub` claim
  itself — it's the bridge between browser-cookie session and the
  canonical sub string.
- Canonical principal namespace (from `specs/4/9-acl-unified.md`
  "Principal namespace" table, backing `auth/acl.go`'s
  `MatchGroups`/`matchPattern`):
  - OAuth sub: `google:114019...`
  - Folder agent: `folder:atlas/eng`
  - Platform identity: `telegram:user/123456`
  - Room identity: `discord:837.../1504...`
  - Role: `role:operator`
  - Wildcards: `**` (any), `google:*` / `folder:**` (namespace)

**Smallest ACL setup to authorize exactly one local user to post
messages:**

Two building blocks exist, both driven by the CLI (`cmd/arizuko/main.go:441-449`,
`runGrant`/`runUngrant` at `cmd/arizuko/main.go:526-546`):

```
# 1. Grant the human's OAuth sub interact (read+send) on their folder.
#    (there is no tier default for `interact` — spec 4/9: "interact and
#    admin have no tier default: always explicit"). This is what
#    `auth.Authorize` needs to allow action="interact" scope="<folder>".
arizuko group <instance> grant github:alice main   # -> admin action row today (CLI wraps `admin`, see below)
```

Note precisely: `runGrant` (`cmd/arizuko/main.go:538-544`) inserts an
`acl` row with `Action: "admin"` (not `interact`) unless the pattern is
`**`, in which case it instead adds `acl_membership(sub, role:operator)`
(`cmd/arizuko/main.go:530-536`). So the **minimal row-set for "exactly
one local user, exactly posting"** is one row:

```sql
INSERT INTO acl (principal, action, scope, effect, granted_by, granted_at)
VALUES ('github:alice', 'admin', 'main', 'allow', 'bootstrap', <now>);
```
— `admin ⊃ interact` in the action lattice (`auth/authorize.go:219-233`,
`actionCovers`), so this one row is sufficient for a local user to send
into folder `main`. Equivalent CLI form: `arizuko group <instance>
grant github:alice main`. This is genuinely the floor — `Authorize`
denies-by-default for `interact`/`admin` with no matching row (no tier
fallback for those two actions; only `mcp:*` falls back to
`grants.DeriveRules`, `auth/authorize.go:100-114`).

Also required (not ACL, but prerequisite state): a `groups` row for the
folder itself (`arizuko group <instance> add <jid> <folder>`,
`cmd/arizuko/main.go:365-401`) — `Authorize`'s tier-default fallback
path additionally needs `opts.Folder`/`opts.WorldFolder` set from a
real group, and message routing needs a `routes` row binding a JID to
the folder (same `add` command, `cmd/arizuko/main.go:395-421` for
non-Discord jids: one `PutRouteRow{Match: "room=" + jid, Target:
folder}`).

## 2. Identity as coordinate system

- **Namespacing**: unified, not per-channel-siloed. Every principal —
  human, agent, channel-room, role — is `namespace:rest`, globbed
  segment-wise on both `:` and `/` (`auth/acl.go:21-51` `matchPattern`/
  `matchSegments`; `auth/authorize.go:158-183` `matchPrincipal`
  additionally globs the `:`-namespace). `specs/4/9-acl-unified.md`
  "Principal namespace" table is canonical: `google:114019...`
  (OAuth), `telegram:user/123456` (channel identity, no OAuth link
  yet), `discord:837.../1504...` (room/route audience), `folder:atlas/eng`
  (agent container), `role:operator` (indirection).
- **Cross-channel identity coherence**: via `acl_membership` — the
  *same* table used for role membership and role hierarchy also carries
  "JID claim" edges: `acl_membership(discord:user/811..., google:114alice)`
  means "this Discord identity IS this Google-OAuth human"
  (`specs/4/9-acl-unified.md` "Membership: roles, JID claims, channels"
  table; mirrored code path: `auth/authorize.go:117-142`
  `expandPrincipals` walks `s.Ancestors(p)` transitively for both the
  caller principal and any `caller.Extra` principals — e.g. the room
  JID, per the doc comment on `Caller.Extra`,
  `auth/authorize.go:16-18`). Concretely: link a Telegram user to a
  Google-OAuth human by inserting one `acl_membership` row; every ACL
  check for the Telegram sub then also sees the human's grants.
- **Implicit principal set at message arrival** (spec 4/9, "the
  gateway expands the caller's principal set to include BOTH"
  `{caller_jid, room_jid}` plus their transitive `acl_membership`
  ancestors) is exactly what `expandPrincipals` implements — one
  `Authorize` call, not per-channel logic.
- **GRANTS.md** is deliberately a thin pointer, not the model itself —
  it names `specs/4/9-acl-unified.md` as the canonical spec and
  `auth/authorize.go`, `auth/policy.go`, `store/acl.go`,
  `store/membership.go`, `store/migrations/0052-acl-unified.sql` /
  `0053-acl-cutover.sql` as the canonical code, explicitly rejecting
  "4-layer" documentation drift.

## 3. Social attach — exposing the same agent on Bluesky (or any channel)

**Registration flow** — self-registration, channel → router (per
`specs/4/1-channel-protocol.md` "Why self-registration" and "Register"
sections):

```
POST /v1/channels/register   (on routd)
Authorization: Bearer <shared-secret>
{
  "name": "bluesky-mybot",
  "url": "http://bskyd:8080",
  "jid_prefixes": ["bluesky:user/", "bluesky:"],
  "capabilities": {"send_text": true, "send_file": true, ...}
}
→ 200 {"ok": true, "token": "<session-token>"}
```
Implemented by `chanreg.Registry.Register` (`chanreg/README.md`
"Public API"). bskyd calls this itself at boot via `chanlib.RouterClient
.Register` (`chanlib/chanlib.go:146-174`), driven by
`ROUTER_URL`/`CHANNEL_NAME` env (`bskyd/README.md` "Configuration":
`BLUESKY_IDENTIFIER`, `BLUESKY_PASSWORD`, `ROUTER_URL`,
`CHANNEL_NAME`). No manual "add channel" CLI step for the adapter
itself — it's process-startup self-registration, matching the doc's
"Anyone can write one in any language. Router doesn't need static
config for channels."

**The operator-facing command that DOES need running** is binding a
specific room/JID to the agent's folder:

```
./arizuko group <instance> add bluesky:user/<did-or-handle> main
```
(general form documented at `INSTALL.md:196-208`: `./arizuko group
<name> add <jid> <folder>`; implemented at `cmd/arizuko/main.go:365-421`,
`cmdGroup` case `"add"`). This is what actually wires "this social
channel JID routes to this agent folder" — the channel *registration*
(bskyd → routd) only tells routd "I own this JID prefix and here's my
`/send` URL"; the *group add* is what tells routd "route inbound from
this specific JID to folder X."

**Channel-id (JID) format**: `<platform>:<scheme>/<id>`, platform-scheme
segment glob-matchable. Concretely for Bluesky:
`bluesky:user/<percent-encoded-did>` (inbound, current),
`bluesky:<did>` (legacy, outbound-only) — `bskyd/README.md`
"Responsibilities": *"Deliver inbound as `bluesky:user/<encoded-did>`
JIDs (legacy `bluesky:<did>` accepted outbound)"*; encoding rationale
at `bskyd/client.go` `bskyUserJID` (DIDs contain `:`, percent-encoded
so `*` in glob patterns doesn't cross `/`). Other adapters: `telegram:
group/<id>`, `discord:<guild_id>/<channel_id>`, `slack:<team_id>/channel/
<channel_id>` (`INSTALL.md:196-206`).

**Incoming-post → turn routing, concretely (bskyd)**:

1. `bskyd/client.go:167-178` `poll` — every 10s, `fetchNotifications`
   calls Bluesky's `app.bsky.notification.listNotifications` (mentions
   + replies), oldest-unread-first (`bskyd/client.go:224-253`).
2. `handleNotification` (`bskyd/client.go:276-322`) builds a
   `chanlib.InboundMsg{ID: n.URI, ChatJID: bskyUserJID(author.DID),
   Sender: <same>, Content: <post text>, Verb: "message"|"reply",
   Topic/ReplyTo: <parent post URI when a reply>, IsGroup: true}` and
   calls `rc.SendMessage(msg)`.
3. `chanlib.RouterClient.SendMessage` (`chanlib/chanlib.go:192`) POSTs
   this to routd's `/v1/messages` with the adapter's own
   `service:bskyd` bearer token (obtained via
   `SetServiceToken`/`/v1/service-token`, same flow as §1).
4. `routd/server.go:387` `handleMessages` — verifies scope
   `messages:write`, resolves the adapter name from the verified
   `sub` (`service:bskyd → bskyd`, `routd/server.go:393-396`),
   validates the adapter owns the JID prefix it's posting under
   (`routd/server.go:410-421`, `s.reg.ByPrincipal(sub).Owns(jid)`),
   idempotency-keys the message id, resolves topic inheritance for
   replies, and (mention-promotion aside) inserts the message and lets
   the routing table (`routes` rows keyed by `room=<jid>` / `room=<jid>
   verb=mention`, written by `group add`) determine which agent folder
   gets a **turn** — the trigger that spawns/continues the agent run.
5. Outbound: routd calls back to bskyd's registered `url` +`/send`
   (`POST http://bskyd:8080/send`, `bskyd/README.md` "Serve `/send`...";
   generic contract at `specs/4/1-channel-protocol.md` "Send message"),
   with `chat_jid`, `content`, optional `reply_to`/`thread_id` — 200
   response means it landed on the platform.

**To attach an *existing* agent/group (not create a new one)**: no new
config needed on the agent side — `group add` just adds another
`routes` row pointing a *different* JID at the *same* `folder`. The
same `folder`'s conversation state (sessions/topics, see §4) is shared
across every channel routed to it unless the operator deliberately
partitions by topic.

## 4. Threads across channels

Thread/conversation keys live in two tables, both under `store/`:

- **`messages`** (`store/migrations/0023-drop-dead-columns.sql:31-49`,
  the current shape after the 0023 rebuild): `id TEXT PRIMARY KEY,
  chat_jid TEXT NOT NULL, sender, sender_name, content, timestamp,
  is_from_me, is_bot_message, forwarded_from, reply_to_id,
  reply_to_text, reply_to_sender, topic TEXT NOT NULL DEFAULT '',
  routed_to, verb, attachments, source`. `chat_jid` is the raw
  channel/room JID (e.g. `bluesky:user/<did>`, `telegram:group/123`,
  or an internal `web:`/`hook:`/bare-folder scheme for local clients —
  `routd/server.go:404-409` comment on ingress JID-ownership). `topic`
  is the thread key *within* a chat/folder.
- **`sessions`** (`store/migrations/0008-topic-sessions.sql:4-9`):
  `PRIMARY KEY (group_folder, topic)`, mapping `(group_folder, topic)
  → session_id` — i.e. the conversation/turn state is keyed by
  **agent folder + topic**, not by `chat_jid`. `chat_jid` decides
  *routing* (which folder a message lands in); `topic` decides *thread
  continuity* once inside a folder.
- **`sessions`** was later extended with topic-lineage columns
  (`store/migrations/0055-topic-lineage.sql`): `parent_topic,
  forked_at, observed_cursor` — supporting explicit thread forking and
  a per-topic "observed since" cursor.

**Coherence across social + local channels**: because `sessions` keys
on `(group_folder, topic)` and NOT on `chat_jid`, two different
channels routed to the *same folder* with the *same topic* string
share one conversation/session — e.g. a Bluesky reply thread and a
local-client thread against the same agent folder are only coherent if
they're given the same `topic` (Bluesky's topic is the AT-URI of the
root post for replies, per `bskyd/client.go:283-286`: `topic =
n.Record.Reply.Parent.URI`). By default, distinct channels naturally
get **distinct topics** (Bluesky topic = post URI; local client's
topic can be anything the caller sets, defaulting to `""` = the
folder's "main" thread) — so social-channel threads and local threads
stay *separate* unless something deliberately unifies the topic
string. `reply_to_id`/reaction-topic-inheritance
(`routd/server.go:449-452`, an inbound with no topic but a
`reply_to` inherits the parent message's topic) is the one place
routd auto-derives topic continuity, but only within the same
`chat_jid`'s reply chain.

## 5. Onboarding (onbod)

**Not required for a purely local single-user deployment.** Two
independent kill-switches:

- `ONBOARDING_ENABLED=0` — onbod's `main.go` exits immediately
  (`onbod/README.md` "Configuration": *"`ONBOARDING_ENABLED` — `0`
  exits immediately"*).
- Even if onbod runs, it only gates a *self-service* admission path —
  it "owns" the `onboarding`, `invites`, `onboarding_gates` tables in
  its own `onbod.db` (`onbod/README.md` "Tables owned"), and its public
  surface (`GET/POST /onboard`, `GET /invite/{token}`) is about
  matching unrouted/new JIDs to gates (`github-org`, `google-domain`,
  `catch-all`) and creating a fresh user world via
  `container.SetupGroup` (`onbod/README.md` "Responsibilities":
  *"Poll `awaiting_message` rows... Match users to gates... Promote
  queued users to `approved`"*). It does **not** own `acl`/`groups`/
  `routes` (those are routd-territory; onbod only cross-writes them
  during world creation/invite redemption, per its own README's
  "Tables owned" section: *"`auth_users`/`acl`/`groups`/`routes` are
  NOT onbod-owned"*).
- For a single pre-provisioned local user (the `arizuko group
  <instance> add` + `grant` flow from §1), there is no "unrouted JID"
  ever hitting onbod's queue in the first place — routing already
  exists, so onbod's admission machinery is simply never invoked. It's
  purely additive for multi-tenant/public self-service onboarding, not
  a gate the core message/auth pipeline passes through.

## Sources consulted

- `auth/README.md`, `auth/authorize.go`, `auth/acl.go`, `auth/identity.go`
- `authd/README.md`, `authd/main.go`, `authd/http.go`
- `GRANTS.md`, `specs/4/9-acl-unified.md`
- `bskyd/README.md`, `bskyd/client.go`
- `chanreg/README.md`, `chanlib/chanlib.go`
- `onbod/README.md`, `onbod/integration_test.go`
- `routd/server.go`, `routd/reads_http.go`
- `core/types.go`
- `store/migrations/0001-initial-schema.sql` (referenced),
  `store/migrations/0008-topic-sessions.sql`,
  `store/migrations/0023-drop-dead-columns.sql`,
  `store/migrations/0055-topic-lineage.sql`
- `cmd/arizuko/main.go` (`cmdGroup`, `runGrant`/`runUngrant`/`runGrants`)
- `groupfolder/folder.go`
- `INSTALL.md`
- `specs/4/1-channel-protocol.md`
