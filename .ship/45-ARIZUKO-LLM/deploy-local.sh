#!/usr/bin/env bash
# Local arizuko deploy for the RSX trading assistant — PROVEN working end-to-end
# on 2026-07-11 (single-box, PROFILE=web). Every layer verified: routing →
# container spawn → ipc bridge → persona injection → Claude Code SDK → reply
# round-trip. The ONLY thing gating a real answer is the Anthropic credential
# (see step 8): without it the agent replies "Not logged in · Please run /login".
#
# Environment quirks this script encodes (learned the hard way):
#  - Daemon containers run as user 1000:1000 (compose hardcodes it). So the
#    instance data dir must be 1000-owned, AND arizuko CLI ops that write there
#    must run as uid 1000 (`sudo -u '#1000'`), matching the containers.
#  - `docker` works for us via the docker group; `docker compose` (v2 plugin)
#    was installed to /usr/libexec/docker/cli-plugins so root/sudo sees it.
#  - HOST_DATA_DIR is REQUIRED: runed spawns arizuko-ant as a *sibling* via
#    docker.sock, so the ipc/group bind-mount paths must be translated from
#    runed's in-container view (/srv/app/home) to the real host path. Unset =>
#    docker auto-creates root-owned dirs => node EACCES on /run/ipc/input.
#  - ASSISTANT_NAME must be a bare token (no spaces / specials) or every daemon
#    crash-loops on "load config".
set -euo pipefail

ARIZUKO_SRC=/home/onvos/app/arizuko
export PREFIX=/home/onvos/.arizuko
INST=rsx
DATA="$PREFIX/data/arizuko_${INST}"
COMPOSE="$DATA/docker-compose.yml"
U1000=(sudo -u '#1000' env "PREFIX=$PREFIX" HOME=/home/onvos)   # run CLI as the container uid

# 1. build the images we actually need (NOT `make images` — it builds every
#    adapter and dies on whapd's npm/git step, which we don't use).
cd "$ARIZUKO_SRC"
GOTOOLCHAIN=auto make build
sudo make agent        # arizuko-ant:latest (Claude Code agent, ~8GB)
sudo make vite-image   # arizuko-vite:latest (webd SPA server)
GOTOOLCHAIN=auto sudo docker build -t arizuko .   # arizuko:latest (authd/routd/runed/webd/proxyd)

# 2. install docker compose v2 system-wide if missing
if ! sudo docker compose version >/dev/null 2>&1; then
  sudo mkdir -p /usr/libexec/docker/cli-plugins
  sudo curl -sSL https://github.com/docker/compose/releases/latest/download/docker-compose-linux-x86_64 \
    -o /usr/libexec/docker/cli-plugins/docker-compose
  sudo chmod +x /usr/libexec/docker/cli-plugins/docker-compose
fi

# 3. create the instance (auto-seeds AUTH_SECRET / SECRETS_KEY / default group)
cd "$ARIZUKO_SRC"
"${U1000[@]}" ./arizuko create "$INST" || true   # idempotent-ish; ignore "exists"

# 4. configure .env (bare ASSISTANT_NAME; web profile; host-path translation;
#    onboarding off; credential placeholder). Edit as uid 1000.
"${U1000[@]}" bash -c "cat >> '$DATA/.env'" <<EOF

# --- MVP web-chat plane ---
WEB_PORT=8095
PROFILE=web
ONBOARDING_ENABLED=0
HOST_DATA_DIR=$DATA
EOF
"${U1000[@]}" sed -i 's/^ASSISTANT_NAME=.*/ASSISTANT_NAME=rsx/' "$DATA/.env"

# 5. persona folder (the agent's $HOME is $DATA/groups/main). PERSONA.md is the
#    canonical persona — copy the tracked one from the sprint dir.
"${U1000[@]}" mkdir -p "$DATA/groups/main"
sudo install -o 1000 -g 1000 -m 644 \
  "$(dirname "$0")/PERSONA.main.md" "$DATA/groups/main/PERSONA.md" 2>/dev/null || true

# 6. own the whole instance by the container uid, then generate compose + tokens
sudo chown -R 1000:1000 "$DATA"
cd "$ARIZUKO_SRC"
"${U1000[@]}" ./arizuko generate "$INST"

# 7. bring the stack up (root reads the 1000-owned .env; containers run as 1000)
sudo docker compose -f "$COMPOSE" up -d --remove-orphans

# 8. register the local web-chat route, then smoke a turn.
#    Run `group add` / `send` from a NEUTRAL cwd (not the arizuko repo) so a
#    relative "groups/" seed path can't clobber the source tree.
cd /tmp
"${U1000[@]}" "$ARIZUKO_SRC/arizuko" group "$INST" add web:main main || true
#
# >>> CREDENTIAL — the agent's Anthropic auth (VERIFIED 2026-07-11) <<<
# runed's readSecrets (container/runner.go:707) reads CLAUDE_CODE_OAUTH_TOKEN /
# ANTHROPIC_API_KEY from runed's OWN container env and injects it into the agent
# as the operator anchor. It is NOT in compose's env passlist, so putting it in
# $DATA/.env does NOTHING (verified: it never reaches runed.env). It must land in
# runed's env_file directly. `generate` rewrites env/runed.env, so append AFTER
# generate (re-append after every regenerate):
#   TOKEN=$(grep -E '^\s*export\s+CLAUDE_CODE_OAUTH_TOKEN=' ~/.bashrc | grep -v '^\s*#' | \
#           tail -1 | sed -E 's/^\s*export\s+CLAUDE_CODE_OAUTH_TOKEN=//; s/^"//; s/"$//')
#   printf 'CLAUDE_CODE_OAUTH_TOKEN=%s\n' "$TOKEN" | "${U1000[@]}" tee -a "$DATA/env/runed.env" >/dev/null
#   sudo docker compose -f "$COMPOSE" up -d --force-recreate runed
# (A host `claude setup-token` OAuth token works; so does ANTHROPIC_API_KEY=sk-ant-…)
"${U1000[@]}" "$ARIZUKO_SRC/arizuko" send "$INST" main "hello — do you see the RSX context?" --wait

# HTTP path (what rsx-term uses): mint a route token once, then POST /chat/{token}.
#
# SPLIT-TOPOLOGY QUIRK (verified 2026-07-11): `arizuko token issue` writes
# route_tokens into $DATA/store/messages.db (cmd/arizuko/token.go → store.Open),
# but webd resolves tokens against $DATA/store/routd.db (webd/main.go →
# store.OpenRoutd) — so a freshly CLI-minted token 404s ("route token not
# found" in webd logs). Until the arizuko CLI is split-aware, copy the rows
# over after minting (as uid 1000, matching the containers):
#
#   TOKEN=$("${U1000[@]}" "$ARIZUKO_SRC/arizuko" token "$INST" issue chat main | awk '/^token:/{print $2}')
#   "${U1000[@]}" python3 - <<'PY'
#   import sqlite3
#   src = sqlite3.connect("/home/onvos/.arizuko/data/arizuko_rsx/store/messages.db")
#   dst = sqlite3.connect("/home/onvos/.arizuko/data/arizuko_rsx/store/routd.db")
#   rows = src.execute("SELECT token_hash, jid, owner_folder, created_at FROM route_tokens").fetchall()
#   dst.executemany("INSERT OR IGNORE INTO route_tokens(token_hash, jid, owner_folder, created_at) VALUES (?,?,?,?)", rows)
#   dst.commit()
#   PY
#   curl -N -X POST "http://localhost:8095/chat/$TOKEN" \
#     -H 'Content-Type: application/json' -H 'Accept: text/event-stream' \
#     -d '{"content":"[RSX CONTEXT]...","topic":"t-<unixms>-rsx-BTC"}'
#
# Then: RSX_TERM_ASSIST="http://localhost:8095/chat/$TOKEN" for the terminal.
