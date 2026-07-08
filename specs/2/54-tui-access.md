# 54 — TUI Access (SSH + Web)

Status: **partial**. SSH forced-command dispatch is implemented
(`scripts/rsx-tui-dispatch`, `scripts/rsx-tui-authorize`). The web
terminal is specified with a config skeleton; its service is deferred.

Give anyone with a registered key their own live `rsx-tui` session
against the deployment, over two front doors — SSH and a browser —
without minting a unix account or a bespoke client per trader.

## Identity model (shared by both paths)

> **Transport note.** `rsx-tui` speaks **protobuf-over-QUIC** only
> (`rsx-tui/src/quic.rs`); the WebSocket client is gone. Identity is now
> carried in-band: on connect the client mints an HS256 JWT for its
> `RSX_TUI_USER` and sends it in the QUIC **auth first-frame**
> (`WireHello { jwt, user }`) before any order. What is deferred is the
> gateway VALIDATING that frame — a QUIC listener that checks the JWT and
> binds the connection to `user` (see `rsx-tui/ARCHITECTURE.md` "Server
> side"). The access model below — authenticate a person, map them to one
> `RSX_TUI_USER`, launch the TUI — is unchanged and transport-agnostic.

A TUI session's identity is these values (see `rsx-tui/src/main.rs`):

- `RSX_TUI_USER` — the u32 the session trades as. The client mints a JWT
  carrying this as its `user_id` claim and sends it in the auth
  first-frame; the gateway validates it (server-side follow-up).
- `RSX_TUI_SYMBOL` — the u32 symbol id the session trades (the TUI is
  single-market; stamped on every order).
- `RSX_GW_ADDR` — the gateway QUIC endpoint to dial (`ip:port`), with
  `RSX_GW_CERT` (the DER cert to trust) and `RSX_GW_SERVER_NAME` (the TLS
  name, default `localhost`).
- `RSX_GW_JWT_SECRET` — the HS256 signing secret the gateway shares.

The whole access problem is: **authenticate a person, map them to one
`RSX_TUI_USER`, then launch `rsx-tui` with that env against the
deployment.** The secret is a server-side value the launcher holds; it
signs the JWT and never travels to the client as a secret (the client
gets only the minted token). This spec never adds a second identity
system — it reuses the JWT the gateway checks (spec 11-gateway,
spec 49-webproto). Both front doors are, at bottom, an authenticated way
to pick `RSX_TUI_USER`.

## SSH forced-command dispatch (implemented)

The git-shell / gitolite pattern applied to `rsx-tui`. One restricted
unix user, `rsx-tui`. Its `~/.ssh/authorized_keys` holds one line per
registered pubkey; each line forces a command that launches *that key's*
session. Registering a trader is appending a line; revoking is deleting
one. No per-trader unix accounts.

### authorized_keys line format

```
restrict,pty,command="/usr/local/bin/rsx-tui-dispatch <user_id>" <pubkey> <comment>
```

- **`restrict`** (OpenSSH 7.2+) is deny-all: no port-forwarding, no
  agent-forwarding, no X11-forwarding, no `~/.ssh/rc`, no pty. It is
  forward-compatible — future OpenSSH restrictions are denied by default.
- **`pty`** re-adds the one capability a full-screen ratatui app needs: a
  terminal. This is the single, deliberate deviation from git-shell
  (which forces `no-pty` because git speaks a pipe). Without a pty
  `rsx-tui` has no screen to draw on.
- **`command="…"`** forces the dispatch wrapper regardless of what the
  client asks to run. The user id is baked into the command line — the
  authorized_keys file *is* the key→user table, so there is no second
  file to drift out of sync.
- **`<pubkey> <comment>`** — the trader's public key and a unique human
  label (e.g. `trader-alice`), which is how a line is found again to
  revoke it.

Example: `scripts/rsx-tui.authorized_keys.example`.

### The dispatch wrapper

`scripts/rsx-tui-dispatch <user_id>` (install to
`/usr/local/bin/rsx-tui-dispatch`):

1. Validates `<user_id>` is a bare non-negative integer — the one input
   the key line controls. This is the trust boundary for the arg.
2. Logs and drops `SSH_ORIGINAL_COMMAND` (whatever the client tried to
   run). `rsx-tui` takes no arguments.
3. Sources `/etc/rsx-tui/env` (override with `RSX_TUI_ENV_FILE`), mode
   `0400`, owner `rsx-tui` — this holds `RSX_GW_JWT_SECRET` and the
   gateway QUIC endpoint (`RSX_GW_ADDR`, `RSX_GW_CERT`). The secret lives
   here, never in authorized_keys; `rsx-tui` mints the session JWT from
   it (the human never sees the token).
4. Exports `RSX_TUI_USER=<user_id>` (and `RSX_TUI_SYMBOL` for the market),
   then `exec rsx-tui` (override the binary with `RSX_TUI_BIN`). The QUIC
   dial + auth first-frame follow from the sourced env.

The client's PTY becomes the TUI; quitting the TUI ends the SSH session.

### Key registration / rotation

`scripts/rsx-tui-authorize`, run as the `rsx-tui` user:

```
rsx-tui-authorize add <user_id> <pubkey-file|-> [comment]   # ssh-copy-id style
rsx-tui-authorize list
rsx-tui-authorize remove <comment>
```

`add` reads a pubkey from a file or stdin and appends a correctly-formed
line. Rotation is `remove <comment>` then `add` with the new key.
Revocation is `remove <comment>`. For fleet management, keep the pubkeys
under version control and rsync the assembled `authorized_keys` — the
file is the whole registry.

### Server setup

```
sudo useradd --system --create-home --shell /usr/sbin/nologin rsx-tui
sudo install -m 0755 scripts/rsx-tui-dispatch /usr/local/bin/
sudo install -m 0755 scripts/rsx-tui-authorize /usr/local/bin/
sudo install -m 0755 <rsx-tui binary> /usr/local/bin/rsx-tui
sudo install -d -o rsx-tui -g rsx-tui -m 0700 /etc/rsx-tui
printf 'RSX_GW_JWT_SECRET=%s\nRSX_GW_ADDR=%s\nRSX_GW_CERT=%s\n' \
  "$SECRET" "$GW_ADDR" /etc/rsx-tui/gateway.der \
  | sudo install -o rsx-tui -g rsx-tui -m 0400 /dev/stdin /etc/rsx-tui/env
```

`nologin` as the login shell is safe: the forced command runs directly,
never a shell. `make tui-ssh-setup` prints these steps and syntax-checks
the wrappers.

### Security properties

- **No shell.** The forced command replaces the shell; there is no prompt
  to escape to. `SSH_ORIGINAL_COMMAND` is dropped.
- **No forwarding.** `restrict` blocks port/agent/X11 forwarding, so a key
  cannot tunnel to internal services (casting/UDP, Postgres) — it can only
  drive a TUI.
- **Bounded identity.** A key can only trade as the `RSX_TUI_USER` its
  line names. Changing that requires write access to `authorized_keys`
  (the `rsx-tui` user / root), which traders do not have.
- **Secret isolation.** `RSX_GW_JWT_SECRET` is in a `0400` root/`rsx-tui`
  file, never in a key line or the client's environment.
- **Trust boundary honored.** This adds no auth to casting — SSH pubkey
  auth gates *external* humans reaching the launcher; internal RSX peers
  remain L3-trusted (spec 4-cast §10.4, CLAUDE.md trust boundaries). The
  client sends the minted JWT in the QUIC auth first-frame; the gateway
  validating it is the pending server-side piece.

## Web terminal (specified; service deferred)

The same TUI in a browser: xterm.js in the page, a PTY backend spawning
`rsx-tui`, one session per browser connection with its own
`RSX_TUI_USER`, fronted with TLS at `rsx.krons.cx`.

```
browser (xterm.js) --wss--> nginx (TLS, rsx.krons.cx) --> ttyd (PTY) --> rsx-tui
```

### Why deferred

The SSH path's identity comes for free from SSH pubkey auth. The web path
has no equivalent until a browser is authenticated and mapped to a
`RSX_TUI_USER` — and the tool that spawns the PTY (ttyd/gotty/wetty) runs
**one** command for **all** connections; it has no built-in per-session
identity. Supplying per-browser identity requires an auth front end and a
spawn-time mapping (below). That is a real service to stand up, wire to
the existing auth, and TLS-front — it cannot be built and verified on the
dev box in this pass, so it is specified, not half-built.

### Design

- **Front:** xterm.js is the terminal; the PTY backend is **ttyd** (single
  static binary, mature, `libwebsockets` — the boring choice over a Node
  or Go daemon). ttyd serves the xterm.js bundle itself; no separate
  front-end build.
- **TLS + WS:** nginx terminates TLS at `rsx.krons.cx` and proxies the ttyd
  WebSocket, forwarding `Upgrade`/`Connection` (the same nginx WS-upgrade
  requirement that bit the old React trade UI, since removed).
- **Per-session identity — the open design point.** ttyd runs one command
  per instance. Two ways to give each browser its own `RSX_TUI_USER`:
  - **(a) Auth proxy + spawn-per-session.** A small front service
    authenticates the browser against the existing auth service
    (`rsx-auth`), resolves the session's `RSX_TUI_USER`, and launches a
    per-session `ttyd --once rsx-tui-dispatch <user_id>` (or an equivalent
    PTY spawn), proxying that browser to it. `rsx-tui-dispatch` is reused
    verbatim — the web path and SSH path share one launcher. This is the
    recommended shape.
  - **(b) One ttyd per user, static route.** A ttyd instance per active
    trader, each pinned to a `<user_id>`, addressed by an
    auth-gated path. Simpler to reason about, does not scale to many
    users, wastes idle processes. Fine for a small demo cohort.
- **Session isolation:** one PTY (hence one `rsx-tui`, one gateway QUIC
  connection, one JWT) per browser connection. ttyd `--once` ties the
  process lifetime to the connection so a closed tab reaps the session.
- **Auth:** the browser is authenticated **before** ttyd — at the nginx /
  auth-proxy layer against `rsx-auth`, never by exposing
  `RSX_GW_JWT_SECRET` to the browser. The browser gets a terminal, not a
  token.

### Config skeleton

ttyd, per-session launch (shape (a)); `<user_id>` comes from the
authenticated session, not the client:

```
ttyd --once --port 7681 --interface 127.0.0.1 \
     --client-option disableLeaveAlert=true \
     /usr/local/bin/rsx-tui-dispatch <user_id>
```

nginx front (WS upgrade is mandatory):

```nginx
location /tui/ {
    proxy_pass http://127.0.0.1:7681/;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "upgrade";
    proxy_set_header Host $host;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
    # auth_request against rsx-auth here — resolve the session -> user id
    # -> per-session ttyd upstream. This mapping is the deferred piece.
}
```

## Open decisions for the founder

1. **Web auth binding.** How does a browser session resolve to a
   `RSX_TUI_USER` — reuse `rsx-auth` sessions (shape (a)), or a static
   per-user ttyd route for a small cohort (shape (b))? Determines whether
   the auth-proxy service gets built now.
2. **Deploy home for the SSH launcher.** The `rsx-tui` unix user +
   `/etc/rsx-tui/env` + installed wrappers belong on the gateway host (or
   a bastion). Not yet in `specs/2/9-deploy.md` — fold in once the deploy
   topology firms up.
3. **User-id provenance.** Today the launcher trusts the `RSX_TUI_USER`
   in the key line / session. If traders should map to real accounts in
   `rsx-auth`, the launcher should resolve the id from there instead of
   from the key line — a tightening, out of scope for this pass.

## Cross-references

| Concern | Spec |
|---------|------|
| Gateway JWT auth | 11-gateway.md |
| WS wire protocol | 49-webproto.md |
| casting trust boundary | 4-cast.md §10.4 |
| Deployment topology | 9-deploy.md |
