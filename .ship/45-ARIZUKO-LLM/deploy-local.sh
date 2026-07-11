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
# >>> LIVE TURN NEEDS THE CREDENTIAL <<<
#   Uncomment ONE in $DATA/.env, then `arizuko generate` + `docker compose up -d --force-recreate runed`:
#     ANTHROPIC_API_KEY=sk-ant-...
#     CLAUDE_CODE_OAUTH_TOKEN=...   # from `claude setup-token`
"${U1000[@]}" "$ARIZUKO_SRC/arizuko" send "$INST" main "hello — do you see the RSX context?" --wait

# HTTP path (what rsx-term uses): mint a route token once, then POST /chat/{token}.
#   TOKEN=$("${U1000[@]}" "$ARIZUKO_SRC/arizuko" token "$INST" issue chat main | tail -1)
#   curl -N -X POST "http://localhost:8095/chat/$TOKEN" \
#     -H 'Content-Type: application/json' -H 'Accept: text/event-stream' \
#     -d '{"content":"[RSX CONTEXT]...","topic":"t-<unixms>-rsx-BTC"}'
